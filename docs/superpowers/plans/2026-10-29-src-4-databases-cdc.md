# `SRC-4` plan: more databases, with live change capture

| | |
| --- | --- |
| Backlog | `SRC-4` (Building, #264) |
| Spec | `docs/core/specs/src-4.md` |
| Feature page | `docs/core/features/databases-and-change-capture.md` (13 decisions, all signed 2026-10-29) |
| ADR | `docs/adr/0016-change-capture-per-connector-on-kafka-connect.md` (Proposed until slice 0) |
| Branch | `feat/data-phase-4-src-4`, off `main`; one pull request, #264 (draft until the last slice) |
| Anchors | Verified at `b551ddbd` (main `25a0e42a` merged into the branch) unless a task says otherwise |
| Planner / reviewer | Claude Opus. Developer: a Sonnet agent per slice (`AGENTS.md`) |

## Decisions already made (do not reopen)

The feature page's thirteen. In short: Kafka Connect started by the API
(D1), Teradata split to `SRC-35` (D2), the five sources (D3), the `SRC-10`
load modes for changes (D4), a change in Bronze within 2 minutes (D5,
measured), lag in seconds per table (D6), an alert at 15 minutes (D7),
Oracle batch completed (D8), setup checks with fixed sentences (D9), built
on what `SEC-14` part B left (D10), ADR 0016 (D11), one CI job per source
(D12), and the order below (D13).

## Machine limits (this VM)

The root disk has under 1 GB free and is shared. **No local `cargo build`,
`cargo clippy`, `cargo test`, `docker build` or `docker compose up`.** Run
`cargo fmt`, `python -m pytest` on pure unit tests, `ruff` and `bun test` on
single files where `node_modules` exists. CI compiles and runs everything;
every handoff names what ran only in CI (rule 7).

## Anchors

| What | Where |
| --- | --- |
| Fixed CDC pipeline | `docker-compose.yml` service `debezium-server` (~:3121) and its properties heredoc; `g4-source-init` (~:3041) |
| CDC settings template | `rust/crates/lakehouse-store/src/cdc.rs:612` `render_debezium_properties_template` (PostgreSQL-shaped; a MongoDB branch at :641) |
| CDC routes and refusals | `rust/crates/lakehouse-api/src/routes/connectors.rs:2615` (properties), :2709 `oracle_cdc_refusal`, :2744 `unbuilt_cdc_template_refusal`, :3880 ingest/run answers `supported:false` for cdc |
| Dial shapes | `rust/crates/lakehouse-store/src/ingest_spec.rs:30` `SqlDriver`, :97 `CdcDial` (slot and publication required), :1679 `supports_column_choice` |
| Connection test | `rust/crates/lakehouse-api/src/connector_probe.rs:396` `probe_dial`, :444 Oracle "not testable here"; `pg_probe.rs` (SEC-14 part B: TLS, SCRAM only, one fixed `SELECT 1`) |
| Batch writer and load modes | `dagster/dispar_orchestrate/adapters/sink.py` (modes :40-100, `_apply_cursor` :761), `adapters/history.py`, `adapters/dedup.py` |
| Kafka consumer pieces | `dagster/dispar_orchestrate/adapters/kafka.py` (`consume_one_batch` :126, `compute_lag` :175, advertised-broker check :117) |
| Oracle batch | `dagster/dispar_orchestrate/adapters/oracle.py`, dispatch `ingest_factory.py:760-800`, gate `ops/g6/g6_oracle_test.py`, job `g6-oracle` (`ci.yml` ~:1447) |
| CDC gate | `ops/g4/g4_test.py`, `ops/g4/docker-compose.g4.override.yml`, job `g4-cdc` (`ci.yml:918`) |
| Measurement workflow to copy | `.github/workflows/g9-upload-large.yml` (`workflow_dispatch`, one job) |
| Gate coverage | `ops/gates/coverage.json`, lint `ops/lint/check_connector_gate_coverage.py` (statement shapes it replays: plain `INSERT INTO connector_type ... VALUES`, `UPDATE ... SET supported/adapter/unsupported_reason = literal WHERE name = '...' [AND ...]`) |
| Load alerts | `rust/crates/lakehouse-api/src/routes/load_alerts.rs` (kinds `connector_failure` ...), `dagster/dispar_orchestrate/pipeline_events.py` |
| Connector page | `src/features/connectors/connector-detail-page.tsx:39` tabs `overview / ingest / schema / tests` |
| Migrations | `rust/migrations/`; **first free number across all branches: 0115** (0112-0114 are taken on other branches). Re-check before each slice that adds one |

## Slice 0: measurement (branch `measure/src-4-cdc`, never merged)

Purpose: prove or refute ADR 0016 before product code (D13). Cut
`measure/src-4-cdc` from `feat/data-phase-4-src-4`. Everything in this slice
lives on that branch except the result file (T0.6), which the developer
commits to `feat/data-phase-4-src-4`.

- **T0.1 Compose override** `ops/src4/docker-compose.src4.override.yml`:
  a broker (`apache/kafka:3.8.0`, KRaft single node, as `kafka-g6`) and a
  Kafka Connect worker from Debezium's own image (`quay.io/debezium/connect`,
  the newest 3.x tag that exists, **pinned by digest**; offsets, config and
  status in Kafka topics, replication factor 1). Source: the stack's own
  `postgres:16` (already `wal_level=logical`), a new schema `src4` with a
  table `orders (id bigint primary key, customer text, amount numeric(12,2),
  status text, updated_at timestamptz)`, `REPLICA IDENTITY FULL`, its own
  publication and slot (never `p5cdc_*`). Credentials `${VAR:?}` in the
  override only. *Check:* `docker compose ... config --quiet` in CI.
- **T0.2 The applier, prototype** `dagster/dispar_orchestrate/cdc_apply.py`:
  reads one topic in batches (reuse `adapters/kafka.py`: the advertised-broker
  check and `compute_lag`), turns Debezium events into rows, writes them
  through `adapters/sink.py` with one of two modes, then commits the Kafka
  offsets **after** the Bronze commit. Event shape: Debezium's
  `ExtractNewRecordState` transform on the connector with
  `delete.handling.mode=rewrite` and `add.fields=op,source.ts_ms`, JSON
  without schemas. Modes:
  - *current row per key*: within one batch only the last event per key
    counts; the batch is applied in **one `pyiceberg` transaction of the
    applier's own**, as `adapters/history.py` does: delete the batch's keys
    from the table, then append the last version of each key whose last
    event is not a delete. A delete event only deletes. Replaying a batch
    gives the same table. The table is still created and evolved through
    `sink`'s helpers. (Amendment 1: `dlt`'s `hard_delete` does nothing on
    this path.)
  - *history*: append every event with `__op` and `__source_ts_ms`.
  A Dagster job `cdc_apply_job` (config: topic, table, key, mode) and a
  sensor that launches it when the topic has unconsumed messages,
  `minimum_interval_seconds=30`. Unit tests (no network): the event-to-row
  mapping for c/u/d/r and a tombstone, the last-per-key rule, and that
  offsets are committed only after a successful write (a fake consumer and a
  failing sink). *Check:* `python -m pytest` on the new test file, locally.
- **T0.3 The harness** `ops/src4/measure_cdc.py` (pure functions unit-tested
  in `ops/src4/test_measure_cdc.py`, no network): registers the connector by
  calling Kafka Connect's REST API directly (the API is not touched in this
  slice), seeds 10,000 rows, waits for the first copy, then, for **each
  mode**:
  1. *Freshness*: 30 inserts, 30 updates and 30 deletes, one every 2 s;
     for each, the time from the source commit to the change being visible
     in Bronze (read through ClickHouse as `g4_test.py` does). Report p50,
     p95, max per kind.
  2. *Correctness*: after the 90, the Bronze table equals the source
     (current mode: same keys, same values, deleted keys absent; history:
     one row per event, in source order per key).
  3. *Stop and catch up*: pause the connector and stop the sensor; write
     108,000 changes at the source (30 per second for an hour's worth,
     written as fast as the source takes them; a mix of 70 % updates, 20 %
     inserts, 10 % deletes); resume both; report the time to catch up and
     check again that Bronze equals the source, with no row missing and none
     twice (history: exactly 108,000 new events).
  4. *Cost*: peak memory of the broker and the Kafka Connect worker
     (`docker stats`), and the job's run time per batch.
  It prints a `GATE-SUMMARY`-style block and writes `src4-result.json`.
  `set -euo pipefail` in any shell step; every expected status is asserted
  (lessons from the `DATA-17` harness: a measured success is never assumed).
- **T0.4 Workflow** `.github/workflows/src4-measure.yml`: `workflow_dispatch`
  and `push` to `measure/src-4-cdc` only; one job on `ubuntu-latest`, timeout
  60 min; brings up the stack plus the override, runs the harness, uploads
  `src4-result.json` and the logs as an artifact. Copy the shape of
  `g9-upload-large.yml`.
- **T0.5 Run it** at least twice (two runs, same code) and keep both run ids.
- **T0.6 Result file** `docs/plans/SRC-4-RESULT.md` on
  `feat/data-phase-4-src-4`: the two runs' numbers in tables, the image
  digests, what was not measured, and a one-line verdict against the targets:
  - D5: p95 of each change kind within **120 s** in both modes;
  - correctness and catch-up: Bronze equals the source after both, **zero**
    missing or duplicated rows;
  - cost: the broker and worker's peak memory, stated (no target; it is
    written into ADR 0016's consequences).
  If any target is missed, the developer stops and reports; the planner
  takes it to the product owner (D13), and no further slice starts.

Out of slice 0: the API, the console, migrations, any change to the fixed
pipeline or to `g4-cdc`, any other database.

## Slice 1: Oracle batch (D8)

Independent of slice 0's verdict, so it may run in parallel with it.

- **T1.1 Connection test for Oracle.** The API has no Oracle client and a
  Rust one needs Oracle's native library. The test runs in the orchestrator,
  as listing does today (`SRC-13`, `routes/connector_discovery.rs` and
  `dagster/dispar_orchestrate/discover.py:666 _oracle`), answering in the
  same fixed `Outcome` sentences. *Developer: read how `SRC-13` reaches the
  orchestrator first; if it cannot carry a connection test without a new
  route, stop and report (the planner decides the route).*
- **T1.2 Column choice** for Oracle: add it to `supports_column_choice`
  (`ingest_spec.rs:1679`) and make `adapters/oracle.py` read only the chosen
  columns; the TS contract follows.
- **T1.3 Incremental loads** for Oracle: the cursor (`sink._apply_cursor`)
  on the Oracle resource; `incremental` and `incremental_dedup` allowed.
- **T1.4 Encrypted dial**: `oracledb` thin mode over TLS (`tcps`) to the
  gate's fixture, with a CA the gate generates (as the SQL Server fixture
  does); the existing `ssl_server_cert_dn` rule (`ingest_spec.rs:1268-1283`)
  kept. Fetch nothing new into git.
- **T1.5 Gate** `g6-oracle`: connection test from the console route (pass,
  wrong password, unreachable), column choice, an incremental reload that
  brings only new rows, all over TLS; `coverage.json` connect step from
  `refused` to real.
- *Acceptance:* `SRC-4-AC6`; `g6-oracle` green and still under 20 minutes.

## Slice 2: the change-capture runtime, PostgreSQL on it (D1, D4, D11)

Starts only after slice 0's verdict is "met". Detailed task list written into
this plan before dispatch (the planner amends this section, under "Plan
amendments", with anchors verified at that commit). Scope:

- The broker and Kafka Connect in `docker-compose.yml` under a `cdc` profile,
  internal network only, `${X:?}` for what must be set; a real `up` proves it
  (rule 8).
- A Kafka Connect client in the API (`kafka_connect.rs`): base URL from
  config, fail closed when unset, an allowlist of the five Debezium connector
  classes, fixed error classes (principle 4).
- Create, update, pause, resume and delete of a change-capture connector call
  it; the connector JSON is rendered from the registry row (ADR 0007's
  renderer, per class). Secrets: a Kafka Connect config provider resolving
  the `secretRef`, never a literal in the JSON (ADR 0016 point 4, settled
  against ADR 0002 in the amendment).
- The applier from slice 0, productionised: one sensor per change-capture
  connector, run records, `SRC-7` alerts, `SRC-8` schema changes, the load
  mode per table chosen in the console.
- `g4-cdc` moved onto this path; `debezium-server`, its sidecar and the
  `__deleted` reader filter removed; ADR 0016 set to Accepted.
- *Acceptance:* `SRC-4-AC1` and `AC2` for PostgreSQL in `g4-cdc`.

## Slice 3: MySQL, then SQL Server (D3, D12)

Amended in detail before dispatch (Amendment 5). Fixtures with the source settings on
(MySQL `binlog_format=ROW`, `binlog_row_image=FULL`; SQL Server Agent on,
CDC enabled on the database and table); the types flipped back from `0066`
with a targeted `UPDATE ... AND supported = false` (a new migration);
`coverage.json`; one gate job each (`g4-mysql`, `g4-mssql`).

## Slice 4: lag, the alert, setup checks (D6, D7, D9)

Amended in detail before dispatch. Lag per table (now minus the source
commit time of the last change applied; "up to date" when the topic has no
backlog); a migration for the per-connector limit, default 15 minutes; a
"Change capture" view on the connector page; alert kind `connector_lag`
(once when crossed, once when back under). Setup checks with a fixed
sentence and the command per missing setting: PostgreSQL by extending
`pg_probe.rs` with fixed statements (never back to `sqlx`, D10), MySQL and
SQL Server in their probes, Oracle and MongoDB in the orchestrator.
*Acceptance:* `SRC-4-AC3`, `AC7`.

## Slice 5: MongoDB (D3)

Amended before dispatch. A replica-set fixture (`mongo:7 --replSet`, one
member), a "MongoDB CDC" type (migration plus `coverage.json`), the Debezium
MongoDB class in the allowlist, its gate job.

## Slice 6: Oracle change capture (D3; last, may be split out)

Amended before dispatch. LogMiner; Oracle's JDBC driver fetched into the
Kafka Connect image at build time (never committed); the fixture in
`ARCHIVELOG` with supplemental logging; DBZ-9681 checked against our
`gvenzl/oracle-free` pin first. If it cannot pass, the planner proposes
splitting it out to its own backlog item; the other four ship.

## Pull request

One pull request, #264, from `feat/data-phase-4-src-4`. Draft until slice 6
(or its split) is done, the full verification block below has passed on the
final commit, and the product owner's browser QA has passed. Title starts
with the backlog ID. Slice 0's branch is never merged and is deleted after
the result file lands.

## Verification before each handoff (the lines for what the slice touched)

```bash
cd rust && cargo fmt --check                      # locally
cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace   # CI only on this VM
bun run typecheck && bun run lint && bun run test # CI; single files locally if node_modules exists
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
python3 ops/lint/check_connector_gate_coverage.py
(cd dagster && python -m pytest dispar_orchestrate -q)
docker compose --profile '*' config --quiet       # CI; plus a real `up` for compose edits (CI)
git status                                        # nothing generated staged
```

## Out of scope

Teradata (`SRC-35`), MariaDB change capture (`SRC-36`), cloud warehouses and
enterprise databases, moving a batch connector to change capture in place,
the dashboards assistant (not a BI feature).

## Plan amendments

(Each slice's detailed task list is added here before it is dispatched.)

**Amendment 1 (2026-10-29, slice 0 developer's stop report, verified by the
planner).** `dlt` 1.30.0's `hard_delete` hint is implemented only in its SQL
merge jobs. The path `adapters/sink.py` uses (filesystem destination,
`table_format=iceberg`) calls `dlt.common.libs.pyiceberg.merge_iceberg_table`,
which accepts only the `upsert` and `insert-only` strategies and has no
delete branch (`site-packages/dlt/common/libs/pyiceberg.py`, around :99-118;
no `hard_delete` anywhere under `destinations/impl/filesystem/`). So T0.2's
current mode applies each batch in one `pyiceberg` transaction of its own
(delete the batch's keys, append the surviving last versions), the
`history.py` pattern; a plain `sink.load_via_sink` call does not cover that
mode. Rejected: keeping deletes as rows marked `__deleted` (fails decision 4);
a `dlt` merge followed by a separate delete (two commits, a crash between them
shows a half-applied batch). The existing `incremental_dedup` mode has the
same limit (it never deletes); not in `SRC-4`'s scope, noted for the backlog.

**Amendment 2 (2026-10-29, slice 1 developer's T1.1 stop report, decided by
the planner).** `POST /api/connectors/{id}/test` is synchronous and writes the
stored health, the audit event and the probe history; discovery (`SRC-13`) is
asynchronous (`discover_job`, the console polls), and its store keeps one row
per `(connector_id, kind)` with `kind` in `list | preview`, so reusing `list`
would overwrite the person's table list. Decision: **a third kind, `test`.**
- A new migration widens the `connector_discovery` kind CHECK to add `test`
  (never edit `0075`); the enum, the result route body and the TS contract
  follow.
- For a driver the API cannot dial (Oracle now; MongoDB's setup checks in
  slice 4), `/test` launches `discover_job` with `kind = test` and answers at
  once with a fixed sentence that the test is running in the orchestrator; it
  never reports a success it has not seen (principle 2).
- When the orchestrator posts the `test` result, the API records it exactly as
  a synchronous test would: stored health, audit event, probe history, the
  same fixed `Outcome` sentences (upstream text classified, principle 4).
- The console shows "testing…" and polls the result, as it does for listing.
- `g6-oracle`'s connect step becomes real (pass, wrong password, unreachable
  host) and `coverage.json` moves from `refused`.
Rejected: a synchronous `/test` that waits for a Dagster run (a run takes tens
of seconds to start; an HTTP request held that long is a timeout waiting to
happen); a new route (a second way to test one connector).

**Amendment 3 (2026-10-29, slice 0 result, verified by the planner from both
runs' `src4-result.json`).** Slice 0 met every target; ADR 0016 is Accepted.
What slice 2 must carry over from it (each is in `docs/plans/SRC-4-RESULT.md`):
- Debezium 3.8 ignores `delete.handling.mode` and `drop.tombstones`; the
  connector renderer writes `delete.tombstone.handling.mode=rewrite`.
- `dlt` renames double-underscore columns: the event columns are `_op`,
  `_source_ts_ms`, `_deleted`, `_offset`, with a test that `dlt`'s naming
  leaves them unchanged.
- A refused schema is not retryable: the applier's op must not retry it (the
  default retry policy hid a refusal for 13 minutes).
- Not yet measured, so slice 2's gate covers it: the secret outside the
  connector JSON (ADR 0016 point 4), a crash or a Kafka Connect restart
  between the Bronze commit and the offset commit, schema change, a source
  above 10,000 rows, several connectors at once.
- Images: `quay.io/debezium/connect:3.8@sha256:d67d678c…` and
  `apache/kafka:3.8.0@sha256:c89f315c…` (full digests in the result file).

**Amendment 4 (2026-10-29): slice 2 in detail.** Anchors verified in the
working tree at `d7e24aad` (the slice 2 survey; line numbers drift, so the
developer finds each by its name). Slice 2 runs after slice 1 is closed, on the
same branch, one task per commit. Planner's decisions (technical, within the
signed decisions; none changes what the product owner signed):

- **S2-a Secrets reach Kafka Connect through a memory-only file the API
  writes.** Managed credentials are sealed with AES-256-GCM (ADR 0002
  addendum 5) and only the API (and the orchestrator) can open them; manager
  references must pass `SECRET_MANAGER_ALLOWED_PREFIXES` at every use
  (addendum 6). So the API resolves the connector's `secretRef` through its own
  allowlisted resolver (`state.rs` `connector_secret_resolver`) and writes the
  plain value to `/run/cdc-secrets/<connector id>` (mode 0400) on a volume
  `lakehouse_cdc_secrets` that is **tmpfs** (`driver_opts: type: tmpfs`), mounted
  read-write in `lakehouse-api` and read-only in `kafka-connect`, nowhere else.
  The connector config names it with Kafka Connect's built-in
  `DirectoryConfigProvider` (`${dir:/run/cdc-secrets:<connector id>}`), so the
  password is never in the connector JSON, the config topic or the REST API's
  answers. tmpfs empties on restart: on start, and on every provisioning call,
  the API rewrites the file for each change-capture connector before it
  touches Kafka Connect. A rotated secret is rewritten and the connector
  restarted. The developer first checks that the pinned
  `quay.io/debezium/connect:3.8` image turns `CONNECT_CONFIG_PROVIDERS*`
  environment variables into worker settings; if not, stop and report. ADR
  0016 point 4 is amended to say this (a plain copy in memory only, the same
  exposure as the worker's own process memory).
- **S2-b The API is Kafka Connect's only client** (`kafka_connect.rs`): base URL
  `KAFKA_CONNECT_URL`; unset means a change-capture connector answers
  `supported: false` with a fixed sentence (fail closed), never a 500. An
  allowlist of connector classes (PostgreSQL now; MySQL, SQL Server, MongoDB,
  Oracle added by their slices). Kafka Connect's error text never reaches a
  response (classified, principle 4). Connector name `lakehouse-<connector id>`,
  topic prefix `cdc.<connector id>`.
- **S2-c Rendering** (`lakehouse-store/src/cdc.rs`): a new renderer produces the
  Kafka Connect connector JSON for PostgreSQL from the registry row and the
  ingest spec: the dial's own `slotName` and `publicationName` (today's template
  ignores them and invents `{slug}_slot`), `table.include.list` from the source
  objects, `snapshot.mode=initial`, `publication.autocreate.mode=disabled`, JSON
  converter without schemas, `ExtractNewRecordState` with
  `delete.tombstone.handling.mode=rewrite` and `add.fields=op,source.ts_ms`
  (amendment 3), the password as the config-provider reference. The Debezium
  Server properties renderer and its test that reads the compose file go.
- **S2-d TLS to the source.** `CdcDial` gains `sslMode` like `SqlDial`
  (`disable | require | verify-ca | verify-full`, default `require`), rendered
  into `database.sslmode`; `SEC-14`'s rule applies (no plain-text dial of a
  stored credential unless the person chose `disable`, shown as such).
- **S2-e Lifecycle.** Create or `PUT ingest-spec` of a change-capture connector
  provisions (write secret, `PUT /connectors/<name>/config`, which creates or
  updates); a new `POST /api/connectors/{id}/pause` (`connector:manage`, in
  `POLICY_TABLE` and `tests/route_auth.rs` both ways) sets `paused_reason` to a
  fixed "paused by a person" and pauses it in Kafka Connect; `resume` lifts that
  reason too and resumes it; the existing pauses (schema change, auto-disable)
  pause it in Kafka Connect as well; `DELETE` removes the Kafka Connect connector
  first, then drops the slot and publication as today, then the secret file.
  Topics are left to the broker's retention (7 days, stated in the limits).
  No migration is needed (`paused_reason` is free text); if one turns out to be,
  the first free number is 0116.
- **S2-f The applier, productionised** (from `origin/measure/src-4-cdc`
  `cdc_apply.py`): one static `cdc_apply_job` with config `connector_id`, one
  sensor `cdc_apply_sensor` (30 s, `default_status=RUNNING` with the reason)
  that lists change-capture connectors from `GET /api/connectors/ingestible`
  (which today filters `cdc` out of what is due: give the sensor its own
  query), skips paused and busy ones, and launches a run for a connector whose
  topics have unread messages (`adapters/kafka.py` `compute_lag`). Each run
  posts run reports (`run_reports.py`) so `SRC-11`'s streak and `SRC-7`'s alerts
  apply unchanged; observes each table's columns through `schema_observer`
  (`SRC-8`), pausing the connector on a "wait" decision; and fails without retry
  on a refused schema (`Failure(allow_retries=False)`, amendment 3; the op keeps
  `DEFAULT_RETRY_POLICY` for everything else, `test_op_source_metadata.py`
  stays as it is).
- **S2-g Load modes for change capture** (D4, without a new enum value):
  `validate_load_modes` accepts for `cdc` exactly `incremental_dedup`, shown as
  "Current row per key" (the default), and `history`; both need a primary key.
  The console's change-capture setup stops hiding `LoadModeFields` and shows
  those two only.
- **S2-h The fixed pipeline goes**, everything in the survey's list:
  `debezium-server`, `debezium-localhost-proxy`, `g4-source-init`'s `p5_cdc`
  seed (replaced by the gate's own source setup), the `lakehouse_debezium_data`
  volume, the Lakekeeper `debezium` principal and its token, `ops/debezium/*`,
  `/debezium-properties` and its console panel (replaced by the Kafka Connect
  connector's state, in fixed words: running, paused, failed), the CI comments
  that avoided `--profile dagster` because of it, and the docs (`OPERATIONS.md`,
  `ARCHITECTURE.md`, README). `.env.staging` is the product owner's file: its
  lines are removed in the branch only, and the handoff names them.
- **S2-i Compose**: `kafka` and `kafka-connect` under a `cdc` profile, pinned by
  digest (amendment 3), no published ports (comment saying why), `${X:?}` for
  what must be set, the tmpfs volume of S2-a, broker retention 7 days.
- **S2-j The `g4-cdc` gate on the new path** (timeout stays 20 min; slowest
  connector job under 20 min, `SRC-9`): create the connector through the API
  (provisions Kafka Connect); two tables, one per mode, 20,000 seeded rows each;
  first copy; insert, update and delete each visible within **120 s** (D5);
  `GET` the connector's config from Kafka Connect and assert the password is
  not in it; pause through the new route, write 20,000 changes, resume, Bronze
  equals the source with nothing missing or twice; restart `kafka-connect`
  during a stream of changes, again equal; a schema change is observed;
  `DELETE` removes the Kafka Connect connector, the slot and the secret file;
  the WAL alert and OIDC steps stay. `coverage.json` follows.

Tasks, in order (one commit each; Rust together, then Python, then console,
then compose and gate; scoped checks per commit, the full block before
handoff):
T2.1 `CdcDial.sslMode` and the Kafka Connect renderer (S2-c, S2-d).
T2.2 `kafka_connect.rs` client and config (S2-b).
T2.3 secret materialisation (S2-a), with the ADR 0016 amendment.
T2.4 lifecycle: provision, pause route, resume, delete (S2-e), `POLICY_TABLE`,
`route_auth.rs`.
T2.5 load modes for `cdc` (S2-g, Rust side).
T2.6 the applier and its sensor (S2-f), with unit tests (no network).
T2.7 the console: load modes, connector state, pause and resume (S2-g, S2-e).
T2.8 compose (S2-i) and removal of the fixed pipeline (S2-h).
T2.9 the gate (S2-j) and `coverage.json`.
T2.10 docs: feature page "Limits", OPERATIONS, CHANGELOG.
*Acceptance:* `SRC-4-AC1`, `AC2` and `AC5` for PostgreSQL in `g4-cdc`; every
other check that passes on `main` passes.

**Amendment 5 (2026-10-29): slice 3 in detail, MySQL then SQL Server.**
Anchors verified at `a1e3dd65` (slice 2 closed); line numbers drift, so the
developer finds each by its name. Slice 3 runs on the same branch, one task per
commit, after slice 2. Planner's decisions (technical, within the signed
decisions; none changes what the product owner signed):

- **S3-a The dial per driver.** `CdcDial` (`lakehouse-store/src/ingest_spec.rs`)
  keeps one shape, but `slotName` and `publicationName` become
  `Option<String>`, and `Dial::parse` checks them per driver:
  - PostgreSQL: both required, as today. Stored PostgreSQL rows parse unchanged.
  - MySQL: `serverId` required, in `1..=4294967295`. `slotName` and
    `publicationName` are refused.
  - SQL Server: `slotName`, `publicationName` and `serverId` all refused.

  A refused or missing field is a 400 with a fixed sentence naming the field.
  ADR 0013 gets a dated note: "required for PostgreSQL" replaces "required".
  The TS contract mirrors serde (`slotName?`, `publicationName?`).
- **S3-b One renderer, three drivers** (`lakehouse-store/src/cdc.rs`
  `render_connect_config`). These keys stay the same for every driver:
  - The password reference.
  - `topic.prefix=cdc.<id>`.
  - The JSON converters without schemas.
  - The `unwrap` transform with `delete.tombstone.handling.mode=rewrite` and
    `add.fields=op,source.ts_ms`.
  - `snapshot.mode=initial` and `decimal.handling.mode=string`.

  Per driver:
  - **MySQL** (`io.debezium.connector.mysql.MySqlConnector`):
    - Connection: `database.server.id` from the dial, `database.include.list`
      from the source objects' first part (`<database>.<table>`), and
      `table.include.list`.
    - TLS: `database.ssl.mode` is `disabled`, `required`, `verify_ca` or
      `verify_identity` for `disable`, `require`, `verify-ca` or `verify-full`.
    - `include.schema.changes=false`, so no topic is named after the prefix
      alone.
  - **SQL Server** (`io.debezium.connector.sqlserver.SqlServerConnector`):
    - Connection: `database.names` is the dial's database, and
      `table.include.list` is `<schema>.<table>`.
    - TLS: `disable` sets `database.encrypt=false`. `require` sets
      `database.encrypt=true` and `database.trustServerCertificate=true`, the
      same meaning `require` has for PostgreSQL. `verify-ca` and
      `verify-full` both set `database.encrypt=true` and
      `database.trustServerCertificate=false`. The SQL Server driver cannot
      check the chain without the host name, so `verify-ca` is stricter here,
      never looser; the code says so.
  - **Schema history** (MySQL and SQL Server only; both refuse to start
    without it):
    - `schema.history.internal.kafka.bootstrap.servers` comes from a new API
      setting.
    - `schema.history.internal.kafka.topic=cdc.<id>.schema-history`.

    The API reads `CDC_KAFKA_BOOTSTRAP` (the variable the code location
    already uses; compose passes it to `lakehouse-api` too). If it is unset,
    a MySQL or SQL Server connector answers the same fixed
    "not configured" sentence that `KAFKA_CONNECT_URL` gets (fail closed).
    PostgreSQL does not need it.

  Both classes join `ALLOWED_CONNECTOR_CLASSES` (`kafka_connect.rs`). Unit
  tests, one per driver, check:
  - The exact keys.
  - That no password value appears anywhere in the JSON.
  - That a table name that is not two plain parts is refused.
  - That each SSL mode maps to the right setting.
- **S3-c Topics.** MySQL writes `cdc.<id>.<database>.<table>`, which is the same
  shape as PostgreSQL. SQL Server writes `cdc.<id>.<database>.<schema>.<table>`.
  `cdc_apply.py` `topic_of` takes the driver from the connector's dial (already
  in `GET /api/connectors/ingestible`), and has one unit test per driver. The
  applier reads only the table topics, never the schema-history topic.
- **S3-d The schema-history topic must never expire.** Debezium creates the
  topic with unlimited retention when it is missing. If it is ever deleted by
  the broker's 7-day retention, the connector cannot restart. The gates check
  the topic's `retention.ms` (and `retention.bytes`) after the first copy. If
  Debezium 3.8 does not create it that way, stop and report; do not raise the
  broker default. `DELETE` cannot remove the topic (Kafka Connect has no topic
  API, and the API has no Kafka admin client). It is a few kilobytes per
  connector, and "Limits" says so.
- **S3-e Source settings, proven by the gates and written in "Limits"** (the
  checks with fixed sentences are slice 4's). The gates use a least-privilege
  user, not `root` or `sa`, so the written grants are proven to be enough.
  - MySQL: `log_bin` on, `binlog_format=ROW`, `binlog_row_image=FULL`. These are
    MySQL 8's defaults; the gate asserts them, it does not assume them. The
    user needs `SELECT`, `RELOAD`, `SHOW DATABASES`, `REPLICATION SLAVE` and
    `REPLICATION CLIENT` (`RELOAD` for the snapshot's short global read lock,
    `snapshot.locking.mode=minimal`, Debezium's default).
  - SQL Server:
    - SQL Server Agent must be running (`MSSQL_AGENT_ENABLED=true` in the
      fixture).
    - CDC must be enabled on the database (`sys.sp_cdc_enable_db`) and on each
      table (`sys.sp_cdc_enable_table`), by an administrator.
    - The user is a non-`sa` login with `SELECT` on the tables and on the `cdc`
      schema, plus `VIEW DATABASE STATE`. The developer confirms that list
      against Debezium 3.8's documentation and the gate. If more is needed,
      write down what and why.
  - SQL Server schema changes are not picked up by themselves. A new column
    needs a second capture instance (Debezium's "online schema update"). The
    gate runs that procedure and checks the column arrives, and "Limits"
    gives the steps.
- **S3-f Fixtures.** The gates reuse `mysql-g6` and `mssql-g6` with the
  `kafka`, `kafka-connect` and `cdc` profile settings of `g4-cdc`, through
  their own overrides (`ops/g4/docker-compose.g4-mysql.override.yml`, `…-mssql…`).
  Credentials are `${X:?}`, as in g6. `mssql-g6` gets `MSSQL_AGENT_ENABLED`
  only in the gate override, so the g6 gates are unchanged. MySQL TLS uses
  `require` (MySQL 8's own self-signed certificate). SQL Server uses `require`
  too: the fixture's private CA is not in Kafka Connect's trust store. The
  verify modes are covered by the renderer's unit tests, as for PostgreSQL,
  and "Limits" says the worker's trust store must hold the source's CA.
- **S3-g The two types become supported.** Migration **0117** (0113 to 0116
  are taken on other branches; re-check before committing) sets
  `supported = true, unsupported_reason = NULL` for `MySQL CDC` and
  `SQL Server CDC`. The update is targeted: `AND supported = false AND
  adapter = 'cdc' AND unsupported_reason = '<0066's exact text>'`. It has a
  why-header. `tests/connector_type.rs`, `ops/lint/test_check_connector_gate_coverage.py`
  and `coverage.json` follow.
- **S3-h The console.** `cdc-dial-form.tsx` shows `slotName` and
  `publicationName` for PostgreSQL only. It shows `serverId` for MySQL only,
  required, empty by default, with a hint ("a number no other MySQL replica
  uses"). The field is never pre-filled with an invented value. Nothing is
  shown for SQL Server beyond the shared fields.
- **S3-i Gates `g4-mysql` and `g4-mssql`** (one CI job each, D12; each under 20
  minutes). The PostgreSQL-free part of `ops/g4/g4_test.py` moves into a shared
  module that all three gates use; `g4-cdc`'s steps and output stay the same.
  Each new gate creates the connector through the API and runs:
  - Two tables, one per load mode, with 20,000 seeded rows each, and the first
    copy.
  - An insert, update and delete each visible within **120 s** (D5).
  - The config from Kafka Connect, checked to hold no password.
  - The schema-history topic's retention (S3-d).
  - A pause through the route, 20,000 changes per table, then a resume: Bronze
    equals the source.
  - A `kafka-connect` restart during a stream of changes: equal again.
  - A schema change observed (for SQL Server, by the procedure in S3-e).
  - `DELETE`, which removes the Kafka Connect connector and the secret file.

  The gate summary table prints a `MySQL CDC` row and a `SQL Server CDC` row.

Tasks, in order (one commit each; Rust together, then Python, then console,
then compose and gates; scoped checks per commit, the full block before
handoff):
T3.1 the dial per driver (S3-a), ADR 0013 note.
T3.2 the renderer for MySQL and SQL Server, schema history, `CDC_KAFKA_BOOTSTRAP`
in the API's config, allowlist (S3-b).
T3.3 migration 0117 and the type tests (S3-g).
T3.4 `topic_of` per driver (S3-c), unit tests.
T3.5 the console dial form (S3-h), contract.
T3.6 the shared gate module; `g4-cdc` unchanged in behaviour (CI proves it).
T3.7 `g4-mysql`: override, gate, CI job, `coverage.json` (S3-d, S3-e, S3-f, S3-i).
T3.8 `g4-mssql`: the same.
T3.9 docs: feature page "Limits" (grants and settings per source, the SQL
Server schema-change steps, the schema-history topic kept), `OPERATIONS.md`,
`CHANGELOG.md`.
*Acceptance:* `SRC-4-AC1`, `AC2` and `AC5` for MySQL and SQL Server in their
gates; `g4-cdc` still passes; every other check that passes on `main` passes.
Stop and report, do not improvise, if: Debezium 3.8 refuses a key above;
the schema-history topic is not created with unlimited retention; SQL Server
needs more than the grants above; or discovery (`SRC-13`) cannot list a
change-capture MySQL or SQL Server source's tables.

**Amendment 6 (2026-10-30): slice 5 in detail, MongoDB, in parallel with
slice 3.** The product owner asked for slice 5 to run while slice 3 is still
open ("jalankan paralel untuk slice 5", 2026-10-30). Slice 5 depends only on
slice 2's runtime, which is closed. Anchors verified at `8cfd0a91` (slice 3's
head that day); line numbers drift, so the developer finds each by its name.

How the parallel work is kept apart:
- **Branch.** `feat/src-4-slice-5`, cut from `feat/data-phase-4-src-4` at
  `8cfd0a91`, worktree `/home/hv/lakehouse-src4-slice5`. CI runs only for
  pull requests into `main` or `lakehouse/**`, so a draft pull request into
  `main`, titled as never to be merged, gives it CI. The branch is merged into
  the phase branch after slice 3 closes, hunk by hunk (rule 9), and that draft
  is then closed. #264 stays the one pull request to `main`.
- **Migration numbers.** 0118 is kept for slice 4 and **0119** is slice 5's
  (0113 to 0117 are taken). Re-check across all branches before committing.
- **Files slice 3 is still changing** (`ingest_spec.rs`, `cdc.rs`,
  `kafka_connect.rs`, `cdc_apply.py`, `cdc-dial-form.tsx`, `ops/g4/*`,
  `ci.yml`, `coverage.json`): slice 5 adds to them and does not rewrite what
  slice 3 wrote, so the merge stays additive.
- **Machine.** No local `cargo build`, `clippy`, `test`, `docker build` or
  `compose up` (the disk has under 6 GB free). `cargo fmt`, pure `pytest` and
  single-file `bun test` locally. CI does the rest.

Planner's decisions (technical, within the signed decisions; none changes what
the product owner signed):

- **S5-a The dial.** A change-capture MongoDB connector is a `cdc` connector
  with its own dial shape, not a fifth `SqlDriver` (MongoDB is not SQL, and
  `SqlDriver` also feeds the batch `sql` adapter). `Dial::parse` for `cdc`
  reads `driver` first: `"mongodb"` parses a new `CdcMongoDial`, anything else
  parses `CdcDial` as today. `CdcMongoDial` (`deny_unknown_fields`, camelCase):
  `driver` (always `mongodb`), `hosts` (explicit `host:port` seeds, each through
  `validate_hostname`, as `MongoDial`), `database`, `user`, `replicaSet` (the
  replica set's name, required: change streams need one), and `sslMode`
  (`CdcSslMode`, default `require`). No `mongodb+srv`, no free-form connection
  string. Its `TargetIdentity` (`SEC-14` re-point rule) is the hosts as a set,
  the database and the replica set name, so changing any of them needs new
  credentials. The TS contract mirrors serde.
- **S5-b The host check.** Whatever slice 2 and 3 do to check a `cdc` dial's
  host before Kafka Connect is asked to dial it is done for **every** entry of
  `hosts`. The connection string the renderer writes names only those hosts.
  Kafka Connect's MongoDB driver discovers the other members of the replica set
  from the server; the developer checks whether Debezium 3.8 accepts
  `directConnection=true` together with change streams on a one-member set. If
  it does, the renderer sets it. If it does not, stop and report: the planner
  decides, and "Limits" will say the worker reaches every member the replica
  set advertises.
- **S5-c The renderer** (`lakehouse-store/src/cdc.rs`, beside slice 3's
  drivers): class `io.debezium.connector.mongodb.MongoDbConnector`;
  `mongodb.connection.string` built from `hosts`, `replicaSet` and the TLS
  setting, with the user and the password as the config-provider reference
  (`DirectoryConfigProvider`, decision S2-a), never a literal;
  `mongodb.authsource` = the database; `collection.include.list` =
  `<database>.<collection>` from the source objects; `capture.mode` =
  `change_streams_update_full` (an update carries the whole document, which the
  current-row mode needs); `topic.prefix=cdc.<id>`; the JSON converters without
  schemas; `snapshot.mode=initial`. The unwrap transform is MongoDB's own
  `io.debezium.connector.mongodb.transforms.ExtractNewDocumentState` with
  `delete.tombstone.handling.mode=rewrite` and `add.fields=op,source.ts_ms`
  (amendment 3's settings, the other class). TLS: `disable` sets
  `mongodb.ssl.enabled=false`; the other three set it `true`, and
  `verify-ca` / `verify-full` also set
  `mongodb.ssl.invalid.hostname.allowed=false` (`require` sets it `true`; say in
  the code that `require` takes the certificate as presented, as for the
  others). The class joins `ALLOWED_CONNECTOR_CLASSES` (`kafka_connect.rs`).
  Unit tests: the exact keys, no password anywhere in the JSON, a collection
  name that is not two plain parts refused, each SSL mode's setting.
- **S5-d Documents into rows** (`cdc_apply.py`). The topic is
  `cdc.<id>.<database>.<collection>`; `topic_of` gets a MongoDB case and a unit
  test. The key is `_id`. Values are turned into columns the same way the batch
  adapter does (`adapters/mongodb.py`: `ObjectId` to its hex string, nested
  documents and arrays as it stores them today), so a batch MongoDB table and a
  change-capture one of the same collection look alike. Reuse its helper; do not
  write a second one (rule 4). Unit tests (no network): insert, update, delete,
  the snapshot read, and an `ObjectId` key.
- **S5-e Load modes.** The two of S2-g, with `_id` as the key filled in by
  default. No other mode is offered.
- **S5-f Connection test and discovery.** The API has no MongoDB client, so the
  connection test of a MongoDB change-capture connector runs in the
  orchestrator through discovery kind `test` (amendment 2's route, as Oracle
  does). Discovery lists the database's collections through `discover.py`'s
  MongoDB code (`_mongo`), reached for the `cdc` adapter when the driver is
  `mongodb`. Fixed sentences, no driver text (principle 4). If either cannot be
  reached without a new route, stop and report.
- **S5-g The type.** Migration **0119** with a why-header: a plain
  `INSERT INTO connector_type (name, adapter, supported, docs_url) VALUES
  ('MongoDB CDC', 'cdc', true, NULL) ON CONFLICT DO NOTHING` (the shape the
  coverage lint replays). `tests/connector_type.rs`,
  `ops/lint/test_check_connector_gate_coverage.py` and `coverage.json` follow.
  The wizard shows it beside the other change-capture types.
- **S5-h Source settings, proven by the gate and written in "Limits"** (the
  checks with fixed sentences are slice 4's). A replica set is required (a
  standalone server has no change streams); sharded clusters are out of scope
  and "Limits" says so. The gate's user is not `root`: the least roles that
  Debezium 3.8's documentation names for change streams on one database
  (expected: `read` on the database, plus what the snapshot and the
  resume-token storage need). The developer confirms the list against the
  documentation and the gate; if more is needed, write down what and why.
- **S5-i The console.** `cdc-dial-form.tsx` shows, for MongoDB only: a hosts
  list (one or more `host:port`), the database, the user, and the replica set
  name (required, empty by default, hint "the name in the server's replica set
  configuration"). Nothing pre-filled with an invented value. The shared
  password and TLS fields stay.
- **S5-j Fixture and gate.** A new service `mongo-cdc-g4` in
  `ops/g4/docker-compose.g4-mongo.override.yml` (not a change to `mongo-g6`,
  so the g6 gate is untouched): `mongo:7` started with `--replSet` and one
  member, an init step that runs `rs.initiate` with the service's own host name
  and polls until it is primary (real readiness), and the gate user.
  Credentials `${X:?}`. One CI job `g4-mongo` (D12, under 20 minutes), shaped
  like `g4-mysql` and using the shared gate module from T3.6. It creates the
  connector through the API and runs: two collections, one per load mode, with
  20,000 seeded documents each, and the first copy; an insert, update and
  delete each visible within **120 s** (D5); the config from Kafka Connect,
  checked to hold no password; a pause through the route, 20,000 changes per
  collection, a resume, and Bronze equals the source with nothing missing or
  twice; a `kafka-connect` restart during a stream of changes, equal again; a
  new field in a document observed as a schema change (`SRC-8`); `DELETE`, which
  removes the Kafka Connect connector and the secret file. The gate summary
  prints a `MongoDB CDC` row.

Tasks, in order (one commit each; Rust together, then Python, then console,
then compose and gate; scoped checks per commit, the full block before
handoff, every Rust and gate check in CI):
T5.1 `CdcMongoDial`, its parse, its `TargetIdentity`, the host check (S5-a,
S5-b).
T5.2 the renderer and the allowlist (S5-c).
T5.3 migration 0119 and the type tests (S5-g).
T5.4 `topic_of`, documents into rows, load modes (S5-d, S5-e), unit tests.
T5.5 connection test and discovery through the orchestrator (S5-f).
T5.6 the console dial form and the contract (S5-i).
T5.7 fixture, gate `g4-mongo`, CI job, `coverage.json` (S5-h, S5-j).
T5.8 docs: feature page "Limits" (replica set required, no sharded clusters,
the roles, what `directConnection` decided), `OPERATIONS.md`, `CHANGELOG.md`.
*Acceptance:* `SRC-4-AC1`, `AC2` and `AC5` for MongoDB in `g4-mongo`; every
other check that passes on `main` passes.
Stop and report, do not improvise, if: Debezium 3.8 refuses a key above; the
`directConnection` question of S5-b has no clean answer; the roles of S5-h are
not enough; the connection test or discovery needs a new route; or a file
slice 3 is changing would have to be rewritten rather than added to.

## Handoff

### Slice 1, Oracle batch (developer, Sonnet, 2026-10-29)

**Commits** (branch `feat/data-phase-4-src-4`, after `130e3ee3`):
`be505478` T1.2 column choice; `5b539988` T1.3 incremental; `bc090b3b` T1.4
encrypted dial and TLS fixture; `500f1534` T1.5 gate; `322ccfc2` R11-lint fix
in the gate. **T1.1 is not done: stopped, see below.**

**T1.1 stop report (the route question).** `SRC-13` cannot carry a connection
test without a planner decision. (1) The orchestrator path is asynchronous
(`POST .../discovery/{kind}` launches `discover_job`, the console polls) while
`POST /api/connectors/{id}/test` (`connectors.rs::test_connection`) is
synchronous and writes `record_test_result`, the audit event and the stored
health. (2) The store keeps one row per `(connector_id, kind)` and `kind` is the
closed set `list|preview` (`connector_discovery::Kind`, CHECK constraint in
`0075_connector_discovery.sql`, the result route's body); "latest wins", so
reusing `list` for a test would overwrite the person's table list. (3) So a
test needs either a third kind (Rust enum, migration for the CHECK, result
route, TS contract) or a new route, and `/test` would have to launch and wait
for the job. Not started. The gate's `connect` step stays `refused` in
`coverage.json` until you decide.

**Findings beyond the plan.**
- *Spelling.* The console takes Oracle column names from the preview in
  Oracle's spelling (`CREATED_AT`); `sql_database` reflects `created_at`.
  Unmapped, `excludedColumns` leaves nothing out (`_leave_out_columns` compares
  exact names) and the cursor is not found (`table.c[...]`; reproduced with
  SQLite in a test). `adapters/oracle.py::in_reflected_spelling` maps
  `excludedColumns`, `primaryKey`, `incrementalKey` with the dialect's
  `normalize_name` (as table names already were); `ingest_factory` applies it
  to the plan and the gate for the Oracle branch only.
- *The existing Oracle TLS mode was clear text.* SQLAlchemy's `makedsn` writes
  `(PROTOCOL=TCP)`; with that descriptor thin mode ignores `protocol` and
  `ssl_server_cert_dn`. Measured with `oracledb==2.5.1` against a loopback TLS
  server: no handshake. `connection_parts` now passes structured keywords
  (`dsn=None`, `host`, `port`, `service_name`, `protocol="tcps"`,
  `ssl_server_cert_dn`); the test `test_a_tls_dial_really_encrypts_and_checks_the_certificate_dn`
  shows the handshake, chain check through `SSL_CERT_FILE`, and `DPY-6002` for a
  wrong DN. The `ssl_server_cert_dn` rule in `ingest_spec.rs` is unchanged.
- *Decision for you, T1.4 fixture.* The gate's TLS fixture is an nginx `stream`
  TLS terminator (`oracle-tls-g6`, pinned by digest) in front of Oracle's plain
  listener, with a CA the gate generates (`oracle-g6-cert-init`). Oracle's own
  TCPS needs a wallet built with `orapki`, which I could not check here. What is
  proven: the product's `tcps` dial, chain and DN. Not proven: Oracle's own TCPS
  endpoint. Written in `oracle-tls-proxy.conf`, the gate docstring and
  `coverage.json`. Say if you want the real wallet attempted.
- `route_auth.rs::ingest_spec_put_refuses_excluded_columns_the_source_cannot_honour`
  used Oracle as its "cannot list columns" case; its case is now Oracle's
  key-column refusal (still 422), the non-listing-source refusal stays in the
  unit test `excluded_columns_are_accepted_only_for_sources_that_list_their_columns`.

**Commands run locally (rule 7)**
- `cd dagster && /home/hv/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate/test_adapters_oracle.py dispar_orchestrate/test_discover.py dispar_orchestrate/test_ingest_factory.py -q`: 164 passed.
- `rustfmt --edition 2024 --check` on the two changed Rust files: clean.
- `python3 ops/lint/check_bare_iceberg_count.py`, `check_intra_package_imports.py`, `check_connector_gate_coverage.py`: OK.
- `docker compose ... config --quiet` on the g6-oracle override: rc 0.
- *Not verified locally* (machine limits): cargo fmt/clippy/test, bun typecheck/lint/test, the compose `up`, the gate. CI is the proof below.

**CI** (PR #264): run `40807707774` (first push, failed only `Repo lints`: the R11 lint matched a docstring; fixed in `322ccfc2`); run `40807993582` on `322ccfc2`: Repo lints, fmt, clippy, build, MSRV, Frontend, G1, G2, G3, G4, G8, G10, SRC-11/12/13/17, Dagster unit tests, images and the image smoke pass; `Rust · G6 Oracle connector` **passed in 4m16s** (the gate itself `PASS (77s)`; `connect` refused-as-expected, discover, load, reload pass; load: 3 rows, `NOTE` left out, over tcps; reload: 2 new rows read, 5 in Bronze). Remaining jobs at the time of writing: see the PR checks.

- *Review round 1, SHOULD-FIX 2 (T1.4).* Oracle's own TCPS was attempted twice and is left unproven: first with `orapki` (the image has no Java, so no wallet), then with a PKCS#12 wallet made by `openssl` (the listener answered `ORA-28759` / `TNS-00566`, failed to open the wallet, and never came up on 2484). The gate is back on the nginx TLS terminator that passed; the honest note stays in `oracle-tls-proxy.conf`, the gate docstring and `coverage.json`.

- *Review round 1, SHOULD-FIX 3 (T1.1).* The Oracle service name is validated as an identifier, `^[A-Za-z0-9_.-]{1,128}$` (letters, digits, `_`, `.`, `-`: the characters of a plain or domain-qualified service name; no wider set was needed). Checked in `ingest_spec.rs` (`is_oracle_service_name`, a fixed 422 sentence) and again in `adapters/oracle.py` (`OracleServiceNameError`, mapped to a rejected setting). Tests: two Rust unit tests and one parametrised Python test. Python: `test_adapters_oracle.py` plus `test_ingest_factory.py`, 108 passed. Rust tests *not verified locally* (disk full); CI is the proof.

(Further entries follow, one per slice.)

### Slice 2, PostgreSQL change capture on Kafka Connect (developer, Sonnet, 2026-10-29)

Commits on `feat/data-phase-4-src-4`, in order:

- `T2.1` to `T2.7`: before this entry (Rust renderer, client, secrets, lifecycle, load
  modes, applier, console). Added in this stretch: `27fa46f1` fixes the TS2353 typecheck
  failure of T2.7 (the saved-object fixture in `connector-ingest-panel.test.tsx` was typed
  `{ name; target }[]`; it now uses the contract's own fields).
- `95457d56` T2.8: the fixed pipeline goes (S2-h) and `kafka` + `kafka-connect` come in
  under the `cdc` profile (S2-i).
- `c1ef8483` T2.9: the `g4-cdc` gate on the new path (S2-j).
- `b9d16250` T2.10: docs. A fix commit between: no Kafka Connect error text in the delete
  refusal (`SRC-4 S2 review BLOCKER-1`, principle 4); and `33d8b758` drops three SEC-11
  `ALLOWED` entries that belonged to the removed handler.

What T2.8 removed: `debezium-server`, `debezium-localhost-proxy`, `g4-source-init`, the
`lakehouse_debezium_data` volume, the Lakekeeper `debezium` principal, token and grant
(compose, `ops/oidc-mock`, `MACHINE_GRANTS` and its test: now four fixed grants),
`ops/debezium/*`, `GET /api/connectors/{id}/debezium-properties` (handler, route, policy
row, tests), `render_debezium_properties*` and its types and the test that read the
compose file (`lakehouse-store/src/cdc.rs`), the g6 MongoDB template step, and the CI
comments that avoided `--profile dagster` because of `debezium-server`. The `ingest/run`
reason for a `cdc` connector now names Kafka Connect, not a compose service.

Compose: `kafka` and `kafka-connect` pinned by tag and digest, no published ports,
`KAFKA_LOG_RETENTION_HOURS=168`, `lakehouse_kafka_data`, `lakehouse_cdc_secrets` (tmpfs,
`mode=1777`, read-write in `lakehouse-api`, read-only in `kafka-connect`),
`CONNECT_CONFIG_PROVIDERS*` for `DirectoryConfigProvider`; `KAFKA_CONNECT_URL` and
`CDC_KAFKA_BOOTSTRAP` as `${X:-}` (empty keeps change capture off). The image's
`CONNECT_*` mapping was checked by the gate (the password is not in the stored config and
the `${dir:...}` reference is).

`.env.staging` (product owner's file; edited in this branch only): lines 30 to 32, the
comment above `LAKEKEEPER_CATALOG_URI` that named `debezium-server` and
`DEBEZIUM_SINK_ICEBERG_URI`, reworded to name no service. No value changed. `.env.example`
gains `KAFKA_CONNECT_URL=` and `CDC_KAFKA_BOOTSTRAP=` (empty) and its Oracle LogMiner
comment is corrected.

The gate (`ops/g4/g4_test.py`): its own source (two tables, 20,000 rows each, one
publication); the connector registered as the console does it; the stored config holds
the `${dir:...}` reference and not the password; the password file exists; first copy;
insert, update, delete within 120 s in both modes; pause, 20,000 changes per table,
resume; a restart of `kafka-connect` during a stream of changes (the CI step restarts the
container when the gate prints `G4-RESTART-KAFKA-CONNECT`, because the runner has no
Docker access); a schema change observed; the WAL alert and OIDC steps as before;
`DELETE` removes the Kafka Connect connector, slot, publication and password file. The job
timeout stays 20 minutes.

Verified by CI on the PR at `33d8b758` (the run before this handoff commit): every check green, including `Rust · clippy`, `Rust · test (stable)` (after the two fixes above; its first run failed `sec11_guard`) and `Rust · G4 change capture into Bronze` (green on its first run; `g4-cdc` also passed on `c1ef8483`). The gate summary table was not read by the developer. **Not verified locally:**
everything. This VM has no room for `cargo`, `docker` or the full suites, so the only local
checks were `rustfmt --check` on the touched Rust files, `ast.parse` and an import of
`g4_test.py`, YAML parse of the compose and workflow files, `bun test` and `tsc --noEmit`
for the one TS fix, and `ops/gates` unit tests (29 passed).

Open items for the reviewer:

- `ORACLE_CDC_LOGMINER_ENABLED` / `Config::oracle_cdc_logminer_enabled` is now read by
  nothing; kept (its doc says so) so that slice 6 can decide, not deleted here.
- `ops/g4` uses `INGEST_ALLOW_INTERNAL_HOSTS=true` on the code location (gate only), as
  the slice 0 measurement did, although `cdc_apply.py` says the guard is not applied to the
  broker; harmless if true, and not proved either way.
- `docs/plans/P5-*.md`, `LAKEHOUSE-FOUNDATION-PLAN.md` and ADRs keep their historical
  `debezium-server` text on purpose (they record what was measured then).

### Slice 2, round 1 fixes (developer, Sonnet, 2026-10-29)

Commit `0d6be072`. `SRC-4 S2 review SHOULD-FIX 1`: `INGEST_ALLOW_INTERNAL_HOSTS` is removed
from the g4 override and its comment now says the applier does not need it. CI on `0d6be072`:
`Rust · G4 change capture into Bronze` concluded `pass` (8m24s, run 40870630314), so the
applier never dials a guarded address and production needs no such flag; the open item in the
slice 2 entry above is closed. `SHOULD-FIX 2`: `replication_metrics.py` (two places) now names
`render_connect_config`, `config.rs` no longer names `debezium-server`, and `docs/OPERATIONS.md`
says an upgraded deployment keeps the Lakekeeper `debezium` principal and grant, revocable by
hand. Other checks on that run are in the PR; only the G4 conclusion was read by me.

## Review

(The planner writes findings here, tagged `BLOCKER` or `SHOULD-FIX`.)

### Slice 1, review round 1 (planner, 2026-10-29, head `b4f11a9a`)

CI run `40809657037` green on `b4f11a9a`; the `g6-oracle` gate printed every
step (discover; load of 3 rows with `NOTE` left out over `tcps`; incremental
reload reading 2 new rows, 5 in Bronze). Read: `adapters/oracle.py`
(`in_reflected_spelling`, structured `tcps` keywords), the gate and fixture.
Two findings beyond the plan were right and are kept: the column-name spelling
map, and the earlier "TLS" dial that sent `(PROTOCOL=TCP)` in clear text.

- **BLOCKER 1 (T1.1).** The Oracle connection test is not built. Build it as
  Amendment 2 says.
- **SHOULD-FIX 2 (T1.4).** The gate proves our `tcps` dial against an nginx
  TLS terminator, not Oracle's own TCPS listener. Try Oracle's listener with a
  wallet once (`orapki` if the pinned image has it, or a wallet generated with
  `openssl` in the PKCS#12 form Oracle reads); if the image cannot do it, keep
  the terminator and leave the honest note where it is (proxy config, gate
  docstring, `coverage.json`).
- **Not a finding:** `g6-oracle` ran in about 3.5 min against 13-16 min in
  `SRC-9-RESULT.md`; the gate's own `PASS (77s)` and its step lines show
  nothing was skipped.

### Slice 1, review round 2 (planner, 2026-10-29, head `89277bad`): closed

CI 39/39 green on `89277bad` (`g6-oracle` 3m58s). BLOCKER 1 fixed: the Oracle
connection test runs in the orchestrator as discovery kind `test` (migration
0115 widens 0075's CHECK without editing it; `/test` answers `pending` with
`ok: false`, never a success it has not seen; the result is recorded once, as a
synchronous test is; a `test` cannot be started through the discovery route).
SHOULD-FIX 2: Oracle's own TCPS listener was tried twice and did not start in
the pinned image; the gate keeps the TLS terminator and says so (proxy config,
gate docstring, `coverage.json`). SHOULD-FIX 3: the Oracle service name is
validated as an identifier in the API and in the orchestrator. No open
`BLOCKER`. Slice 1 is closed.

### Slice 2, review round 1 (planner, 2026-10-29, head `33d8b758`)

Checked by the planner, not taken from the handoff: CI run on `33d8b758` all
green (39 checks), and the `Rust · G4 change capture into Bronze` job log read
line by line. It shows 20,000 + 20,000 rows after the first copy; insert,
update and delete visible in 23 to 32 s (target 120 s) in both modes; Bronze
equal to the source after a pause with 20,000 changes per table and after a
`kafka-connect` restart during a stream; the added column reached Bronze and
is listed as a schema change; the Kafka Connect config holds only the file
reference; `DELETE` removed the connector, slot, publication and password
file; WAL alert and OIDC steps pass. Diff `3a7604ca..33d8b758` read against
S2-a, S2-b, S2-h, S2-i, S2-j: compose blocks pinned by digest, no published
ports, tmpfs secrets volume read-only in `kafka-connect`; the `debezium-
properties` route is gone from the router, `POLICY_TABLE` and the tests; the
three removed SEC-11 `ALLOWED` entries match no line any more; the Lakekeeper
`debezium` principal is gone from `MACHINE_GRANTS`. Not re-run locally (disk,
no cargo or docker on this VM): CI is the evidence.

- **BLOCKER-1 (fixed, `3b21ec7b`).** `remove_connect_connector` put Kafka
  Connect's error text into the 409 body (principle 4, SEC-11). Now logged with
  the connector id; the response text is fixed. Closed.
- **SHOULD-FIX 1 (T2.9).** The gate sets `INGEST_ALLOW_INTERNAL_HOSTS=true` on
  the code location, and `ops/g4/docker-compose.g4.override.yml` says it is so
  "the applier may dial" the broker, but `cdc_apply.py` (lines 75 to 78, 328)
  says the broker dial goes around `ssrf_guard` on purpose, so the flag should
  not be needed. A gate switch that production does not set can hide a
  production failure. Drop the flag from the g4 override and let CI run the
  gate: if it passes, the flag is gone and the comment with it; if it fails,
  name in the handoff what dialled the internal address and why production
  would not need it (or fix it), and correct the comment to say that.
- **SHOULD-FIX 2 (T2.8, rule 1).** Comments still name code that is gone:
  `dagster/dispar_orchestrate/replication_metrics.py` lines 288 and 313 name
  `lakehouse_store::cdc::render_debezium_properties` as the owner of the slot
  name (now `render_connect_config`, which takes the dial's slot and
  publication); `rust/crates/lakehouse-api/src/config.rs:632` lists
  `debezium-server` as an example service; `connectors.rs:3588` and
  `.github/workflows/ci.yml:1239` say "the removed `debezium-server`" (fine as
  history, keep). Fix the first two.
- **Note, no change asked.** A deployment upgraded from `main` keeps the
  Lakekeeper `debezium` principal and its warehouse grant (nothing removes
  them; no service holds its token any more). Harmless; `OPERATIONS.md`
  should say it can be revoked by hand. Add one sentence with SHOULD-FIX 2.
- **Note.** `Config::oracle_cdc_logminer_enabled` read by nothing: slice 6
  decides, as the handoff says.

### Slice 2, review round 2 (planner, 2026-10-29, head `803735c0`): closed

SHOULD-FIX 1: `INGEST_ALLOW_INTERNAL_HOSTS` dropped from the g4 override; the
gate passes without it, so production (which never sets it) needs it no more
than the gate does. SHOULD-FIX 2: `replication_metrics.py` and `config.rs`
name only code that exists; `OPERATIONS.md` says the leftover Lakekeeper
`debezium` principal and grant can be revoked by hand. Checked by the planner:
CI run on `803735c0` success, 34 of 34 jobs (G4 change capture into Bronze
among them), Security 5 of 5. No open `BLOCKER`. Slice 2 is closed.
