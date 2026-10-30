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

Amended in detail before dispatch (Amendment 7). Lag per table (now minus the source
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

**Amendment 7 (2026-10-30): slice 4 in detail, lag, the lag alert, setup checks.**
(Amendment 6 is slice 5's, on `feat/src-4-slice-5`, written by session
lakehouse-35; it arrives with that branch's merge.) Anchors verified at
`070cf04e` (slice 3 closed). Line numbers drift, so the developer finds each
anchor by its name. Slice 4 runs on this branch, one task per commit. The
planner's decisions are technical, within the signed decisions D6, D7, D9 and
D12; the MySQL binlog number in S4-f is the planner's, and the product owner
is told:

- **S4-a Scope.** This slice covers lag and the lag alert for every
  change-capture connector, and setup checks for PostgreSQL, MySQL and
  SQL Server. It leaves out:
  - **MongoDB's setup check:** slice 5 adds it on top of S4-f's mechanism
    (agreed with lakehouse-35).
  - **Oracle's setup check:** it moves to slice 6. Oracle change capture
    does not exist before slice 6, and a check for a type nobody can create
    would be dead code.
- **S4-b Where lag comes from.** The pattern of `pipeline_lag.rs` is reused:
  the orchestrator observes, and the API decides.
  - **Backlog per table.** On every tick (30 s), `cdc_apply_sensor` posts
    each table's backlog (unread messages on its topic, `adapters/kafka.py`
    `compute_lag`) for **every** change-capture connector. That includes
    paused, busy and failed connectors; a failed connector is exactly the
    case the alert exists for.
  - **Last applied change per table.** After each Bronze commit, the applier
    posts, per table, the source commit time of the last change it applied:
    the largest `_source_ts_ms` in the batch.
  - **The route.** Both posts go to one route,
    `POST /api/connectors/{id}/capture/observations`. It uses the same
    service-token auth the orchestrator's run reports use, has a row in
    `POLICY_TABLE`, and is asserted both ways in `tests/route_auth.rs`. The
    body is `{tables: [{table, backlog?, lastSourceTsMs?}]}`; a table the
    connector does not select is refused.
- **S4-c What lag is (D6), one pure function.** The function takes `now` as a
  parameter and is unit-tested like `run_lag_check`. Per table:
  - **"Up to date", lag 0:** the backlog is 0 and Kafka Connect says
    `running`. The table's `caught_up_at` is then set to `now`.
  - **Otherwise, a number:** lag is now minus the later of the last applied
    change's source time and `caught_up_at`. Taking the later of the two
    stops a table that was quiet for a day, then gets one change, from
    reading as a day behind; the true lag is never larger than this. The code
    comment says so.
  - **"Not known yet":** neither time exists.
  - **"Not known since <time>":** the newest observation is older than 2
    minutes (the orchestrator is down). The function never shows a number it
    has not seen (principle 2).
  - **A connector paused by a person** shows "paused" and no number.

  The connector's lag is its worst table's lag.
- **S4-d The lag alert (D7).** Per connector, `lag_alert_minutes` is a whole
  number from 1 to 1440, default 15. A person with `connector:manage` changes
  it through `PUT /api/connectors/{id}/lag-limit` (in `POLICY_TABLE` and
  `route_auth.rs`, both ways).
  - **The kind.** A new `AlertKind::ConnectorLag` (`connector_lag`), scoped
    by connector like `ConnectorFailure`.
  - **Delivery** goes through `evaluate_connector_event`.
  - **Firing.** Each observation post re-evaluates the connector. When its
    lag first passes the limit, one alert fires: the connector, the worst
    table, the lag in minutes and the limit, in fixed words. When the lag
    comes back under the limit, a second alert fires, saying it is back
    under. An open flag on the connector keeps each to once per episode.
  - **Paused by a person:** no evaluation.
  - **Console:** the alert rule editor lists the kind.

  Limit: if the orchestrator is down, nothing posts and no lag alert fires
  ("Limits" says so; the S4-c read still shows "not known since").
- **S4-e Storage, migration 0118** (reserved for this slice; re-check that it
  is free across `git branch -r` before committing). It has a why-header and
  adds:
  - `connector_capture_table`: `connector_id`, `table_name`,
    `last_source_ts`, `caught_up_at`, `backlog`, `observed_at`, with primary
    key `(connector_id, table_name)` and `ON DELETE CASCADE` from `connector`.
  - On `connector`: `lag_alert_minutes integer NOT NULL DEFAULT 15` with a
    `CHECK` for 1 to 1440, and `lag_alert_open boolean NOT NULL DEFAULT false`.
- **S4-f Setup checks in the connection test (D9).** For a change-capture
  connector, `POST /api/connectors/{id}/test` adds `setupChecks`, a list of
  `{setting, status: "ok" | "missing" | "unchecked", message, command?}`.
  - **Fixed text.** Every message and command is a fixed string from one
    table in the code. A `missing` check names the setting and the command
    that sets it. An `unchecked` check says why it could not be read (usually
    a permission). Server text never reaches the answer (principle 4).
  - **The connector can still be saved** whatever the checks say.
  - **Fixed statements only, no value in SQL.** Values to compare (the
    publication name, the selected tables) are compared in Rust against what
    a fixed statement returns.
  - **PostgreSQL** extends `pg_probe.rs`'s own client with fixed statements
    (D10, never back to `sqlx`):
    - `SHOW wal_level` must be `logical`.
    - The role must have replication: `rolreplication` or `rolsuper` of
      `current_user` in `pg_roles`.
    - The dial's publication must exist (`SELECT pubname FROM pg_publication`).
    - The publication must hold each selected table (`pg_publication_tables`).
  - **MySQL:**
    - `@@log_bin` on, `@@binlog_format` = `ROW`, `@@binlog_row_image` = `FULL`.
    - `SHOW GRANTS FOR CURRENT_USER()` must hold `REPLICATION SLAVE` and
      `REPLICATION CLIENT` (or `ALL PRIVILEGES`).
    - Binlog retention: `@@binlog_expire_logs_seconds` 0 (never expires), or
      at least **7 days**. 7 days is the broker's own retention (S2-i), the
      window in which a stopped connector can still catch up; it is the
      planner's number and the product owner is told.
  - **SQL Server:**
    - `is_cdc_enabled` for `DB_NAME()` in `sys.databases`.
    - `is_tracked_by_cdc` for each selected table.
    - SQL Server Agent running, through `sys.dm_server_services`. That needs
      `VIEW SERVER STATE`, which the documented login does not have, so
      without it the check is `unchecked` with that reason. It is never
      reported as a pass, and the login is never widened.
- **S4-g The console.** A **"Change capture"** tab on the connector page
  (`connector-detail-page.tsx` tabs), only for change-capture connectors,
  holds:
  - The Kafka Connect state in fixed words, and pause and resume (moved here
    from the ingest panel, not copied).
  - Each table's lag as S4-c words it.
  - The lag limit editor.

  The read uses the existing capture-state route (`routes/connector_runs.rs`),
  extended with the tables and the limit. The connection test result lists
  the setup checks: missing first, each command copyable.
- **S4-h Gates (D12).** Every change-capture gate (`g4-cdc`, `g4-mysql`,
  `g4-mssql`), through the shared module:
  - Reads the capture route after the first copy and expects "up to date"
    for both tables.
  - Runs one setup check that fails on a missing setting:
    - PostgreSQL: a test with a publication name that does not exist.
    - MySQL: a second login without `REPLICATION CLIENT`.
    - SQL Server: a third table not enabled for CDC, selected.

    Each expects `missing` with the fixed sentence, and the rest `ok`, or
    `unchecked` for the SQL Server Agent check.

  **`g4-cdc` only (AC7):**
  - Set the limit to 1 minute.
  - The CI step stops `kafka-connect` (the marker pattern of the restart
    step), and the gate writes changes.
  - Expect one `connector_lag` alert instance and a lag above 60 s on the
    capture route.
  - Start `kafka-connect` again, wait until caught up, and expect the
    "back under" alert, with exactly two instances in total.

  Each gate stays under 20 minutes.

Tasks, in order (one commit each; Rust together, then Python, then the
console, then the gates; scoped checks per commit, the full block before
handoff):
T4.1 migration 0118 and its store functions (S4-e).
T4.2 the lag function, the observations route, `ConnectorLag`, the
evaluation, the lag-limit route, `POLICY_TABLE`, `route_auth.rs` (S4-b, S4-c,
S4-d).
T4.3 the capture-state route extended (S4-c, S4-g's read).
T4.4 setup checks for PostgreSQL, MySQL and SQL Server (S4-f).
T4.5 Dagster: the sensor's backlog posts and the applier's last-change posts,
with unit tests and no network (S4-b).
T4.6 the console: the "Change capture" tab, setup checks in the test result,
the alert kind in the rule editor (S4-g).
T4.7 the gates (S4-h), `coverage.json` if a column is added.
T4.8 docs: feature page "Limits" (lag's meaning, the orchestrator-down limit,
the 7-day binlog check, the Agent check's permission), `OPERATIONS.md`,
`CHANGELOG.md`.
*Acceptance:* `SRC-4-AC3` (setup check) and `AC7` (lag and one alert) for the
three databases, `AC7`'s alert in `g4-cdc`. All three gates still pass, and
every other check that passes on `main` passes.
Stop and report, do not improvise, if:
- a setup check needs a statement with a value in it, or a grant beyond the
  documented ones;
- `pg_probe.rs`'s client cannot run a statement that returns rows;
- the sensor cannot see a failed connector's topics;
- the alert cannot be made to fire exactly once per crossing.

**Amendment 9 (2026-10-30): slice 6 in detail, Oracle change capture.**
(Amendments 6 and 8 are slice 5's, on `feat/src-4-slice-5`.) Anchors verified
at `0fede55d`. Facts checked outside the repo on 2026-10-30 (sources in the
planner's notes; each one the developer re-checks where a task depends on it):

- **The image runs Debezium 3.7.0.Final, not 3.8.** The digest pinned at
  `docker-compose.yml:3164` (tag `3.8`) has `DEBEZIUM_VERSION=3.7.0.Final`
  in its config; Debezium's own `connect/3.8` Dockerfile sets the same.
  Every "Debezium 3.8" in the repo is corrected by name (principle 5):
  `docs/plans/SRC-4-RESULT.md:26,120,134`, `lakehouse-store/src/cdc.rs:454,682`,
  and this plan's earlier amendments get a one-line correction note.
- **The Oracle JDBC driver is already in that image**
  (`kafka/connect/debezium-connector-oracle/ojdbc11-23.26.1.0.0.jar`, with
  `orai18n`), shipped with the connector since DBZ-7364. Decision 8's
  "fetched into the image at build time" and the feature page's "does not
  ship Oracle's JDBC driver" are corrected: no image build, no download, the
  pinned image as it is. LogMiner needs no `xstreams.jar`.
- **DBZ-9681** (the connector could not parse the version banner of
  `gvenzl/oracle-free`, "Oracle AI Database 26ai Free …") is fixed in
  3.2.7, 3.3.3 and 3.4.0; 3.7.0 includes the fix. The gate proves it.
- **`source.ts_ms` is the change time of the redo row, not the commit time**
  (`source.commit_ts_ms` holds that). The applier keeps `source.ts_ms`, as for
  the other drivers: the lag stays an upper bound (S4-c), never smaller.

Decisions:

- **S6-a Type.** A new connector type `Oracle CDC` (`adapter = 'cdc'`,
  `supported = true`), migration **0120** (`0118` is slice 4's, `0119`
  slice 5's; re-check every remote branch before writing it), the shape
  `check_connector_gate_coverage.py` replays, a `coverage.json` row for the
  new gate. The batch `Oracle` type is unchanged.
- **S6-b Dial.** `CdcDial` with `driver = "oracle"`: `database` is the
  container database's name (`database.dbname`, `FREE` in the gate), new
  optional `pdbName` (`database.pdb.name`; absent for a non-container
  database), both checked as Oracle identifiers with the existing
  `is_oracle_service_name` rule; `slotName`, `publicationName` and
  `serverId` are refused for Oracle, as for SQL Server. `Dial::parse`
  refuses `pdbName` for the other drivers.
- **S6-c Encryption.** Oracle's JDBC driver does TLS only over TCPS with a
  Java trust store, and slice 1 could not get Oracle Free's own TCPS
  listener up (`ORA-28759`). So for Oracle change capture:
  - `sslMode = require` (the default) means Oracle native network encryption
    required: `driver.oracle.net.encryption_client=REQUIRED`,
    `driver.oracle.net.encryption_types_client=(AES256)`,
    `driver.oracle.net.crypto_checksum_client=REQUIRED`,
    `driver.oracle.net.crypto_checksum_types_client=(SHA256)`. Encrypted,
    server not authenticated: the same posture `require` has for the other
    drivers, and the console label says so.
  - `disable` sends none of these, labelled "not encrypted".
  - `verify-ca` and `verify-full` are refused by `Dial::parse` for Oracle
    with a fixed sentence (unsupported, honestly; rule 14). The planner adds
    `SRC-37` "Oracle change capture over TLS with certificate checking" to
    the backlog. The console offers only the two modes for Oracle.
  - The gate proves encryption is on: as `SYS`,
    `v$session_connect_info.network_service_banner` for the connector's
    sessions names the AES256 encryption adapter.
- **S6-d Renderer.** `oracle_entries` in `lakehouse-store/src/cdc.rs`
  beside the other three: `io.debezium.connector.oracle.OracleConnector`,
  `database.hostname/port/user/dbname`, `database.pdb.name` when set, the
  password as `${dir:/run/cdc-secrets:<id>}`, `table.include.list` built
  the way the other drivers build it (escaped, anchored), the
  schema-history keys (`schema_history_entries`, required as for MySQL and
  SQL Server), the S6-c keys, and nothing else: `log.mining.strategy` and
  `snapshot.mode` keep their defaults (`online_catalog`, `initial`). The
  class joins `ALLOWED_CONNECTOR_CLASSES` (`kafka_connect.rs`).
  `CdcSpecError::UnsupportedDriver` stays for no driver; remove the variant
  if nothing returns it.
- **S6-e Names.** Oracle stores unquoted names in upper case, so tables are
  selected and matched in Oracle's spelling (`SCHEMA.TABLE`), and the topic
  is `cdc.<id>.<SCHEMA>.<TABLE>` (no database part; `topic_of` needs no
  Oracle branch: say so in its doc). The Bronze table name follows the rule
  the batch Oracle adapter already uses (`adapters/oracle.py`
  `in_reflected_spelling`), so a batch and a change-capture connector on the
  same table land under the same spelling. Table listing for an Oracle
  change-capture connector uses the orchestrator's existing Oracle
  discovery (`discover.py` `_oracle`, Amendment 2's route); if the CDC dial
  is not wired to it, wire it, no second lister.
- **S6-f Connection test.** As batch Oracle (Amendment 2): `/test` launches
  the orchestrator test with the change-capture dial (encryption as S6-c,
  in `oracledb` thin mode's own native-encryption settings, or stop and
  report if thin mode cannot require it), connecting to the container
  database as the change-capture user.
- **S6-g Setup checks** (decision 9). **Waits for slice 5's merge**: it
  builds on slice 5's mechanism (S8-c, S8-d: the orchestrator test returns
  a closed set of facts, the API turns them into `SetupCheck`s in
  `cdc_setup_check.rs`, an absent or unknown fact is `unchecked`, fixed
  text only). Oracle's facts, read with fixed statements:
  - `ARCHIVELOG`: `SELECT log_mode FROM v$database`.
  - Minimal supplemental logging: `SELECT supplemental_log_data_min FROM
    v$database` (`YES` or `IMPLICIT` is ok).
  - All-column logging per selected table: in the PDB (`ALTER SESSION SET
    CONTAINER` with the validated `pdbName`, or a fixed query on
    `cdb_log_groups` filtered in Python, never a value in SQL),
    `log_group_type = 'ALL COLUMN LOGGING'` for each selected table.
  - LogMiner grants: `SELECT privilege FROM session_privs` and `SELECT role
    FROM session_roles`, compared in Python with the documented list
    (`LOGMINING`, `SELECT ANY TRANSACTION`, `SET CONTAINER`, `CREATE
    SESSION`, `FLASHBACK ANY TABLE` or per-table `FLASHBACK`, `SELECT ANY
    TABLE` or per-table `SELECT`, `EXECUTE_CATALOG_ROLE`,
    `SELECT_CATALOG_ROLE`, `CREATE TABLE`, `LOCK ANY TABLE`, `CREATE
    SEQUENCE`); the fact is the closed set of missing names.
  - Commands: `ALTER DATABASE ADD SUPPLEMENTAL LOG DATA;`, `ALTER TABLE
    "S"."T" ADD SUPPLEMENTAL LOG DATA (ALL) COLUMNS;` (identifiers quoted,
    `"` doubled), `GRANT … TO "<user>" CONTAINER=ALL;`; `ARCHIVELOG` has no
    one-line command (it needs a restart), so its message names the steps
    and gives no command.
- **S6-h Fixture and gate** `g4-oracle` (D12), on `ops/g4/cdc_gate.py`
  like the other three:
  - `gvenzl/oracle-free` pinned by digest (the `g6-oracle` pin, or a newer
    one if that one fails, with the reason in the override). `ARCHIVELOG`
    and minimal supplemental logging are set by an idempotent script in
    `/container-entrypoint-startdb.d` (checks `log_mode` first; `shutdown
    immediate`, `startup mount`, `alter database archivelog`, `alter
    database open`), with `db_recovery_file_dest_size` set for the CI disk.
    The common user `c##dbzuser` with exactly the documented grants (no
    `DBA`), its tablespace in the CDB and the PDB, two tables with
    all-column logging, and a third table without it for the setup-check
    case. Readiness is polled, not assumed (Oracle restarts once).
  - The gate asserts what the other three assert: first copy, insert,
    update and delete within 120 s (D5, measured and printed), both load
    modes, a pause and a `kafka-connect` restart without loss, lag `up to
    date` for both tables, the setup checks (the gate source's exact list of
    setting names ok; the third table's all-column logging missing alone),
    the encryption banner (S6-c), and the delete removing the connector.
  - A new column: assert it if `online_catalog` captures it; if it does not,
    stop and report (the planner decides between `hybrid` and a "Limits"
    line), never a quiet skip.
  - The job stays under 20 minutes; the Oracle image is large, so the
    image pull is cached as `g6-oracle` does.
- **S6-i Console.** The `Oracle CDC` type: the driver preset from the type
  (as the other `… CDC` types), a "PDB name" field, the TLS select with the
  two Oracle modes and its hint, the review step, the brand mark (the
  existing Oracle mark), the setup checks shown as for the others.
- **S6-j Clean up.** `ORACLE_CDC_LOGMINER_ENABLED`
  (`config.rs:313-326`, `:1512`, its tests, `.env.example`, compose, the
  comment in `oracle-dial-form.tsx:15-16`) is read by nothing since slice 2:
  remove it and say so in `CHANGELOG.md`. A deployment that still sets it
  loses nothing.
- **S6-k Docs.** The feature page: decision 8's wording corrected (the
  driver ships in the image), "Limits" for Oracle (no certificate checking,
  `SRC-37`; `ARCHIVELOG` needs a restart; archived logs must be kept as long
  as a stopped connector may need to catch up; Debezium leaves its
  `LOG_MINING_FLUSH` table in the user's schema after the connector is
  deleted; `source.ts_ms` is the change time), the documented grants;
  `docs/OPERATIONS.md`; `CHANGELOG.md`; the 3.7.0 corrections above.

Tasks, in this order (Rust together, the setup check last):
- **T6.1** Migration 0120, `CdcDial` (`pdbName`, Oracle's refusals, S6-c's
  mode refusal), the renderer and the class allowlist, unit tests per key.
- **T6.2** Orchestrator: the Oracle change-capture dial in the test and in
  discovery (S6-e, S6-f); Bronze naming; unit tests.
- **T6.3** Fixture and gate `g4-oracle`, `ci.yml` job, `coverage.json`.
  Push after it and read the gate log before going on.
- **T6.4** Console (S6-i).
- **T6.5** Setup checks (S6-g), only after slice 5 is merged into this
  branch; add their lines to `g4-oracle`.
- **T6.6** Clean-up and docs (S6-j, S6-k).

Stop and report (do not improvise):
- the connector does not start against the pinned Oracle Free image
  (DBZ-9681 or anything else) after reading the worker log;
- `ARCHIVELOG` cannot be set in the container, or the gate cannot finish
  under 20 minutes;
- native encryption cannot be required from the JDBC or the thin driver;
- a grant beyond the documented list is needed;
- a setup check needs a value inside SQL;
- the new column is not captured (S6-h).
If the gate cannot pass, the planner proposes splitting Oracle change
capture into its own backlog item, and the other four sources ship (D13).

**Amendment 10 (2026-10-30, slice 6 developer's stop report, decided by the
product owner).** `python-oracledb` thin mode (2.5.1 pinned, and 26.1.0) has
no native network encryption, so the orchestrator cannot test an Oracle
change-capture dial with the encryption the connector uses (S6-c). The
product owner chose, on 2026-10-30, **to test through Kafka Connect**, and
signed the change to decision 9 for Oracle below. This replaces S6-f and
S6-g, and changes S6-e's table listing.

- **S10-a Connection test.** For an Oracle change-capture dial, `/test`
  asks Kafka Connect to validate the rendered config, with the same
  renderer, Java driver and S6-c keys the connector uses:
  `PUT /connector-plugins/<class>/config/validate`. Kafka Connect stays the
  API's only path to Kafka Connect (ADR 0016):
  - Add a `validate_config` method in `kafka_connect.rs`, with the same
    class allowlist check as `put_config`.
  - Run the host check (`resolve_checked`) exactly as the create path does
    before anything dials.
  - Write the secret file the way the create path does, and reference it as
    `${dir:…}`. If Kafka Connect's validate does not resolve config
    providers, stop and report; never put the password in the request.
  - Classify the answer into the existing fixed `Outcome` sentences, never
    passing on the text: wrong login (`ORA-01017`), unreachable or no such
    service (`ORA-12541`, `ORA-12514`, connect timeout), the server refusing
    encryption (`ORA-12660`, a new fixed sentence), anything else "the
    connection test failed". Log the classified kind with `tracing`, never
    the text.
  - Kafka Connect not configured gives the honest "change capture is off
    in this deployment" answer the other drivers give.
  - The orchestrator routing for an Oracle `cdc` dial in `connector_probe.rs`
    (T6.1, local commit `8f68a5cf`) is reverted.
- **S10-b Setup checks for Oracle (decision 9 amended for Oracle, signed
  2026-10-30).**
  - **In the connection test:** Oracle's three settings are listed as
    `unchecked`, with the fixed message "Checked when change capture
    starts: see the Change capture tab." The settings are `ARCHIVELOG`,
    supplemental logging, and the LogMiner grants.
  - **When the connector or a task is FAILED:** the API turns the task trace
    into a closed set of `SetupCheck`s, by matching Debezium 3.7.0's own
    messages in Rust:
    - `ARCHIVELOG` off;
    - minimal supplemental logging off for the database;
    - all-column logging missing for a named table, where the table name
      must be one of the connector's selected tables or it is not used;
    - a missing LogMiner privilege.
  - The checks show on the Change capture tab, with the S6-g commands
    (identifiers quoted, `"` doubled).
  - Cite each matched message with its file and line in the Debezium 3.7.0
    source.
  - A trace that matches nothing keeps today's fixed failure sentence. No
    trace text is shown (principle 4).
  - No orchestrator facts are involved, so **T6.5 no longer waits for slice
    5**. It reuses `SetupCheck` and `cdc_setup_check.rs` from slice 4.
- **S10-c Tables.** Listing tables in the orchestrator would dial
  unencrypted too, so an Oracle change-capture connector's tables are
  entered by hand as `SCHEMA.TABLE`:
  - Each name is checked as two Oracle identifiers, in Oracle's spelling.
  - The wizard says plainly that the names are not checked against the
    source until capture starts.
  - A name that does not exist captures nothing and its lag reads "not
    known yet". The feature page's "Limits" says so.
  - Use the wizard's existing manual entry if there is one; otherwise add one
    for this type only.
- **S10-d Gate (`g4-oracle`, replaces the S6-h setup-check lines).**
  - The test step passes; a wrong password gives the classified sentence.
  - The encryption banner, as S6-c.
  - A second connector that selects the table without all-column logging:
    its failed state lists exactly that table's missing logging, with the
    command. If Debezium 3.7.0 does not fail for it, or its message carries
    no table name, stop and report.
  - `ARCHIVELOG` and the grants are covered by unit tests on the message
    patterns, since the gate's source has them set.
- **T6.2** shrinks to the orchestrator's Bronze naming and `topic_of`'s doc
  (S6-e). **T6.5** is S10-b. The other tasks and the stop-and-report list
  stand, plus: validate does not resolve config providers (S10-a), and the
  Debezium failure message names no table (S10-d).

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

### Slice 3, MySQL and SQL Server (developer, Sonnet, 2026-10-30)

**Commits** (branch `feat/data-phase-4-src-4`, after `3ed4f1f0`): T3.1 `cb2af122`; T3.2 `75e387a9`
(fix `b750d116`: rustfmt and the split of `render_connect_config`); T3.3 `9af53a17`; T3.4 `3e0bd675`;
T3.5 `3aa06f77`; T3.6 `59d3683f`; T3.7 `2b4bb807`; T3.8 `c5991139`; T3.9 `7ba1a19a`; gate fixes
`a95919d7`, `13e066fb`, `0f64da9d`, `09684d9d`, `8cfd0a91`. `e4bf8dac` merges `origin/main`: the PR was
`CONFLICTING` (one generated line of `docs/core/BACKLOG.md`; main's line taken whole), and a PR in
that state gets no CI at all.

**CI on `8cfd0a91`** (run 40999921257): all 41 checks `pass`, including `Rust · test (stable)`
(37m36s), `Rust · clippy` (3m02s), `Rust · fmt`, `Dagster · unit tests`, `Repo lints`, and the
three change-capture gates, whose summary tables I read in their job logs:

| Gate | Job | Time | Table row | Freshness, insert/update/delete, current and history (s) |
|---|---|---|---|---|
| `g4-cdc` (PostgreSQL) | 123887153465 | 9m08s | `g4 pass pass pass pass` | 40, 45 / 32, 32 / 36, 37 |
| `g4-mysql` | 123887153622 | 7m23s | `g4-mysql pass pass pass pass` | 17, 21 / 34, 34 / 29, 29 |
| `g4-mssql` | 123887153528 | 6m47s | `g4-mssql pass pass pass pass` | 17, 21 / 25, 30 / 27, 31 |

All three are inside D5's 120 s. Each new gate: first copy 20,000 + 20,000 rows; pause, 20,000 changes
per table, resume, Bronze equal to the source (MySQL 49 s, SQL Server 39 s); `kafka-connect`
restarted during a stream of changes, equal again; a schema change observed; `DELETE` removes the
connector and its password file. The jobs are inside the 20-minute target (7 to 9 minutes of gate;
the images job's 13 minutes is the stack's, as for the other gates).

**Findings.**

- *Amendment 5 stop-and-report items.* (1) Debezium 3.8 refused no key: the MySQL keys and the SQL
  Server keys (including `database.encrypt` and `database.trustServerCertificate`) were accepted and
  both connectors ran. (2) The schema-history topic is created with `retention.ms` =
  `9223372036854775807` (`Long.MAX_VALUE`, Debezium's own "forever") and `retention.bytes` = -1, read
  from the broker. The first version of the check wanted -1 for both and failed on that; the check
  now accepts -1 or `Long.MAX_VALUE` and fails on any value that expires. This is the planner's
  call to confirm: the invariant (the topic never expires) holds, the spelling differs from the
  plan's "-1". (3) SQL Server: with the login of the plan (`SELECT` on the tables and on `cdc`,
  `VIEW DATABASE STATE`) the snapshot and the streaming worked and the gate passed, but the worker
  logs `WARN ... Exception caught while calling sys.sp_cdc_help_jobs: SQLServerException: Caller is
  not authorized to initiate the requested action. DBO privileges are required`
  (`SqlServerConnection.getCdcCapturePollingInterval`, called from the
  `SqlServerStreamingChangeEventSource` constructor). Debezium catches it and goes on; Microsoft's
  documentation says `sys.sp_cdc_help_jobs` "requires membership in the db_owner fixed database role".
  No more than the listed grants was needed for the gate; no `db_owner` was granted. What the warning
  costs (the polling interval Debezium assumes) was not measured. (4) Discovery lists both sources'
  tables (`discover` passes for both types).
- *Beyond the plan.* The PR was `CONFLICTING` with `main` for a while and ran no CI; the merge is above.
  A migration number: 0117 was free on every remote branch when committed.
  `ops/lint/check_connector_gate_coverage.py` now replays `WHERE unsupported_reason = '...'` (it tracks
  each row's reason), with a test; the plan named the shape, not the lint change.
- *Test changed, not weakened* (rule 2): `ingest_spec::target_identity_tests::a_cdc_dial_is_re_pointed_by_driver_host_port_or_database_only`
  built a MySQL dial that S3-a now refuses; it is rewritten with a valid dial per driver and asserts
  the same thing (identity = driver, host, port, database).

**Not verified locally** (no cargo, docker, or pytest on this VM): every Rust change, the Dagster unit
test for `topic_of`, and the three gates were verified only by the CI run above. Locally: `rustfmt
--check`, parse and import checks, `bun test src/features/connectors src/lib/connectors` (469 pass),
`tsc --noEmit`, `bun run lint` (0 errors), `ops/lint` (24) and `ops/gates` (29) unit tests,
`ops/g4/test_cdc_gate.py` (6), `scripts/ci/tests/test_required_gate.sh` (56).

**Open for the reviewer.** Whether Long.MAX_VALUE is accepted as "unlimited" (above); whether the
`sp_cdc_help_jobs` warning is acceptable or `db_owner` / a documented alternative is wanted; the TS
`CdcDial` contract still has no `sslMode` (the dial form does not offer it; not in the plan).

### Slice 3, round 1 fix (developer, Sonnet, 2026-10-30)

`SRC-4 S3 review SHOULD-FIX 1`, commit `e08c6644`, console only: `CdcDial` has `sslMode?`
(`"disable" | "require" | "verify-ca" | "verify-full" | string`); `cdc-dial-form.tsx` has a "TLS mode"
select for all three drivers (default `require` and always sent, `disable` labelled "Disable (not
encrypted)", a hint that the verify modes use the worker's trust store and, for SQL Server, that
Verify CA also checks the host name; a stored value the select does not list is kept as an extra
option); the review step shows "TLS mode". Tests in `dial-forms.test.tsx`: default sent as `require`
for each driver, a chosen mode kept on edit (and changed to `disable`). Locally: `tsc --noEmit` clean,
`bun test src/features/connectors` 414 pass, `bun run lint` 0 errors. CI on `e08c6644`: all 41 checks
`pass` (run for that commit, including the three change-capture gates and `Frontend`).

### Slice 4, lag, the lag alert and the setup checks (developer, Sonnet, 2026-10-30)

**Commits** (branch `feat/data-phase-4-src-4`, after `36c282b8`): T4.1 `473e4b21`; T4.2 `f0722fc4`;
T4.3 `bc061438`; T4.4 `93b742b3` (CI fixes `de913e60`, test lints; `65915e81`, T4.1 test times);
T4.5 `4da94629`; T4.6 `25617bda`; T4.7 `c3990e57`; T4.8 `f8765906`. `510052be` merges `origin/main`
(the PR was `CONFLICTING` again, so no CI ran on `65915e81`; one generated line of
`docs/core/BACKLOG.md`, both rows kept).

**CI on `510052be`** (run 41013826932): all 41 checks `pass`, including `Rust · test (stable)`
(39m44s, 8,642 tests passed, 0 failed), `Rust · clippy` (2m31s), `Frontend` (3m45s),
`Dagster · unit tests`. Gate numbers, read in the job logs:

| Gate | Job | Time | Lag after first copy | Setup checks |
|---|---|---|---|---|
| `g4-cdc` | 123930559369 | 13m43s | `g4_cdc.orders up_to_date, g4_cdc.orders_history up_to_date` | `wal_level`, `replication role`, `publication`, `publication tables` all ok; other connector: `publication` missing alone, rest ok |
| `g4-mysql` | 123930559290 | 7m24s | both `up_to_date` | `binary log`, `binlog_format`, `binlog_row_image`, `replication privileges`, `binlog retention` ok; other login: `replication privileges` missing alone |
| `g4-mssql` | 123930559286 | 7m11s | both `up_to_date` | CDC on database and both tables ok, `SQL Server Agent` = `unchecked`; third table: `CDC on table dbo.orders_plain` missing alone |

`g4-cdc` lag alert line: `limit 1 min, kafka-connect stopped: lag 64s, 1 instance, still 1 after more ticks;
started again: ... up_to_date, alert closed, 1 instance listed (15-minute dedupe)`. The lag passed 60 s
after 60.2 s of waiting, the instance appeared 5 s later. Freshness stayed inside D5 (PostgreSQL 26 to 45 s,
MySQL 26 to 49 s, SQL Server 27 to 39 s).

**Mismatch with Amendment 7 (S4-h), stated rather than worked around.** The plan says "exactly two
instances in total" (the crossing and the back-under). The Alerts list dedupes a rule's instances for
15 minutes (`overview::insert_from_fired_rule_with_delivery`), and both events happen within minutes in the
gate, so only one row is listed. The back-under alert is sent (the flag is compare-and-set and delivery
follows it) but is not a second row. The gate therefore asserts: one instance after the crossing and
still one after more ticks over the limit, `alertOpen` true, then up to date and `alertOpen` false. It does
not prove the second delivery arrives (the sink is a dead address, as the WAL rule's). Options for the
reviewer: accept; or change the dedupe for this kind (touches `overview.rs`, outside this slice); or give the
gate a webhook receiver (the address check refuses compose-internal addresses).

**Beyond the plan, or decided by the developer inside it.**

- *Detached delivery.* `announce` runs the flag flip and the delivery in a `tokio::spawn` task and awaits
  it, so an orchestrator post that times out cannot cancel the handler between the flip and the delivery
  (that would leave the flag open and the alert unsent for good). A failed delivery puts the flag back.
- *Templates in commands.* The check texts are fixed; the `command` of a missing check fills a fixed template
  with values this build already holds (the dial's publication and login, the connector's selected tables),
  quoted for the engine. They are shown, never executed. S4-f says "fixed string"; this is the reading that
  lets the command name the table. The statements that read the facts have no value in them.
- *Table checks omitted.* When the PostgreSQL publication does not exist, no "publication tables" check is
  listed (the publication's command already names every table), so the gate's "the rest ok" holds.
- *The PostgreSQL checks* use a new `pg_probe::connect_and_fetch` (SCRAM and TLS only, rows read as text
  from `DataRow`); `authenticate_and_select` was split into `authenticate` and `run_query` with the same
  behaviour for `SELECT 1`. No stop-and-report condition was hit: the client runs row-returning statements.
- *SQL Server visibility.* The check reads `sys.tables`, which shows only tables the login can see; a table
  the login cannot see reads as missing CDC. The documented login has `SELECT` on the selected tables.
- *New steps are not columns of the SRC-9 table.* `lag`, `setup checks` and `lag alert` print their numbers
  and fail the gate by name but are not added to `coverage.json` (no column was added).
- *Ingest tab.* The pause button and the capture panel were moved to the new tab; on the Ingest tab a paused
  change-capture connector says "Resume it on the Change capture tab". The Ingest tests for the moved panel
  moved with it (`connector-change-capture-panel.test.tsx`).
- *Migration number.* 0118 was free on every remote branch (0119 is slice 5's).

**Local checks.** `cargo fmt --check` (clean). Python: `pytest dispar_orchestrate/test_cdc_apply.py` 55 passed
(also `test_ingest_run_reports.py`, `test_schema_observer.py`: 73 passed); `ops/g4/test_cdc_gate.py` 10;
`ops/lint` 24 and `ops/gates` 29 unit tests; `scripts/ci/tests/test_required_gate.sh` 56 and
`test_detect_change_scope.sh` 33; `check_intra_package_imports.py` and `check_bare_iceberg_count.py` OK.
Console: `tsc --noEmit` clean; `bun test src` 2,954 passed, 1 skipped, 0 failed; `bun run lint` 0 errors.

**Not verified locally** (no cargo build, clippy, test, docker on this VM): every Rust change, the three
gates and the `Rust · test` suite were run only by CI (above). The first CI run of the Rust changes failed on
three test-only clippy lints and on a store test that compared nanoseconds with Postgres microseconds; both
fixed (commits above) and green on `510052be`. The MySQL `SHOW GRANTS` and `CAST(... AS CHAR)` decoding and
the tiberius row reads were first exercised by the gates.

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

### Slice 3, review round 1 (planner, 2026-10-30, head `72746884`, code `8cfd0a91`)

Checked by the planner, not taken from the handoff: CI run 40999921257 on
`8cfd0a91` success, 36 of 36 jobs; Security success. The three gate logs read
line by line: `g4-mysql` and `g4-mssql` each seed 20,000 rows per table, load
both, show every insert, update and delete within 120 s (MySQL 17 to 34 s,
SQL Server 17 to 31 s), report the schema-history topic with
`retention.ms=9223372036854775807, retention.bytes=-1`, keep Bronze equal to
the source after a pause and after a `kafka-connect` restart, and see the new
column; `g4-cdc` is unchanged (32 to 45 s). Diff `3ed4f1f0..8cfd0a91` read for
the slice's own files (the `main` merge `e4bf8dac` brings SEC-22, reviewed in
#187): the dial check per driver (S3-a), the renderer split into shared keys
plus one helper per driver with schema history only for MySQL and SQL Server
and the bootstrap list validated (S3-b), the three classes on the allowlist,
`topic_of` per driver (S3-c), migration 0117 targeted on 0066's exact text
(S3-g), `CDC_KAFKA_BOOTSTRAP` passed to `lakehouse-api` with the fail-closed
comment. Not re-run locally (no cargo or docker on this VM): CI is the evidence.

Planner decisions on the handoff's two questions:
- **P1, SQL Server and `db_owner`: option (a), no `db_owner`.** Debezium's call
  to `sys.sp_cdc_help_jobs` fails with the planned grants, is logged as a
  warning, and Debezium goes on; the gate measured snapshot and streaming
  with exactly those grants, within D5. Granting `db_owner` for one
  informational read would give the connector login the right to change the
  database, against least privilege. The feature page's "Limits" already says
  so. If the product owner wants it otherwise, it is a one-line change in the
  documented grants, not in code.
- **P2, "unlimited" retention.** Accepted as built: Debezium 3.8 writes
  `Long.MAX_VALUE` for `retention.ms` and `-1` for `retention.bytes`, both
  never expire; the check fails on any finite retention. S3-d's "-1" was the
  planner's error.

Findings:
- **SHOULD-FIX 1 (S2-d, missed in the slice 2 review).** The TypeScript
  `CdcDial` contract has no `sslMode`, so the console cannot show or choose a
  change-capture connector's TLS mode: every connector made in the console is
  `require`, `verify-ca` and `verify-full` cannot be chosen, and an edit
  cannot keep a mode set through the API. Add `sslMode` to the contract
  (`"disable" | "require" | "verify-ca" | "verify-full" | string`, camelCase
  as serde), a select in `cdc-dial-form.tsx` for all three drivers (default
  `require`; `disable` labelled plainly as not encrypted, `SEC-14`'s rule; a
  one-line hint that for SQL Server `verify-ca` checks the host too, S3-b),
  the review step showing it, and `dial-forms.test.tsx` cases (default sent as
  `require`, a chosen mode kept on edit). Console only; no Rust change.

### Slice 3, review round 2 (planner, 2026-10-30, code `e08c6644`): closed

SHOULD-FIX 1: `CdcDial.sslMode` is in the TypeScript contract; the dial form
has a "TLS mode" select for all three drivers, `require` by default and always
sent, `disable` labelled "not encrypted", the SQL Server hint on `verify-ca`,
an unlisted stored value kept; the review step shows it; two tests cover the
default and a mode kept on edit. Checked by the planner: CI on `e08c6644`
success (the three change-capture gates among the jobs), Security success.
No open `BLOCKER`. Slice 3 is closed; slice 5 (branch `feat/src-4-slice-5`,
session lakehouse-35) may merge into this branch now, hunk by hunk.

### Slice 4, review round 1 (planner, 2026-10-30, head `cec91312`, code `510052be`)

Checked by the planner, not taken from the handoff: CI run 41013826932 on
`510052be` success, Security success. The gate logs read: all three gates
show both tables `up_to_date` after the first copy and exactly one failing
setup check on the second connector (`publication`; `replication
privileges`; `CDC on table dbo.orders_plain`), SQL Server Agent `unchecked`
with the documented grants; `g4-cdc` crossed the 1-minute limit at 64 s,
listed one `connector_lag` instance, and closed the alert after
`kafka-connect` started again. The diff `070cf04e..510052be` read for the
slice's own commits (the `main` merge brings QS-2, reviewed in #275).
Migration 0118 is free on every remote branch, additive, with its
why-header. No values reach SQL sent to a source; no server text reaches a
response; both routes are in `POLICY_TABLE` and asserted both ways. Not
re-run locally (no cargo or docker on this VM): CI is the evidence.

Planner decision on the handoff's AC7 question: **option 1, accepted with
changes.** The 15-minute dedupe of the Alerts list stays (it belongs to
every alert kind, outside this slice), and no in-compose webhook receiver
is added (the address check rightly refuses internal addresses). "Once per
crossing" rests on the compare-and-set flag and its unit tests; the gate
proves the flag goes true then false. SHOULD-FIX 10 removes the assertion
that cannot fail. The product owner's acceptance run checks both deliveries
with a real webhook (AC7).

Findings (no `BLOCKER`):

- **SHOULD-FIX 1 (rule 1), `lakehouse-api/src/capture_lag.rs:25`.** "The
  number is never larger than the true lag" is backwards: anchoring on the
  later of the two times makes the number an upper bound (caught up at
  1000, change at 1020 not applied, now 1030: shown 30 s, true 10 s). Say
  "never smaller than the true lag", as S4-c meant.
- **SHOULD-FIX 2 (principle 2), `routes/capture_lag.rs:115-119`.** A
  `lastSourceTsMs` in the future is accepted, and because `last_source_ts`
  only moves forward (`GREATEST`) it sticks: the table reads "behind 0 s"
  and no crossing fires until the wall clock catches up. Decision: the
  route stores `min(lastSourceTsMs, now)`, logs a `warn` with the connector
  and table when it caps, and the test
  `a_change_time_in_the_future_never_gives_a_negative_lag` asserts the cap.
- **SHOULD-FIX 3, a false alert pair after a resume.** During a person's
  pause no report is `running`, so `caught_up_at` stays at its pre-pause
  value; after resume, one tick with backlog 0 while Kafka Connect still
  says `Starting` gives lag = the whole pause, so any pause longer than the
  limit sends a crossing and, one tick later, "back under". Decision: a
  person's pause does not count as lag. The resume route sets
  `caught_up_at = GREATEST(caught_up_at, now())` for every row of the
  connector, in the same transaction that clears the pause. The feature
  page's "Limits" says: after a resume the lag counts from the resume;
  changes made during the pause show as backlog until applied. A unit or
  store test covers it.
- **SHOULD-FIX 4 (principle 2), `cdc_apply.py:830-848`.** A topic that does
  not exist yet counts 0 and is posted, so a table whose first copy has not
  started reads "Up to date" and `caught_up_at` is set. Leave a missing
  topic out of the lag post (unmeasured, so the console shows "not known
  yet"); keep 0 only for the sensor's own run decision. A unit test covers
  it.
- **SHOULD-FIX 5, `connector-change-capture-panel.tsx:56-58`.** Pause and
  Resume render only when `GET …/change-capture` succeeds with
  `supported: true`, and the Ingest tab now points there. With that GET
  failing (store or Kafka Connect config unreadable), a paused connector
  has no Resume button anywhere, although the resume route does not need
  that GET. Render the Pause/Resume control from `pausedReason` in the
  error and unsupported branches too (state shown as unknown), and test the
  error branch.
- **SHOULD-FIX 6 (principle 2), `cdc_setup_check.rs:350`.** MySQL
  `binlog_expire_logs_seconds = 0` reads "never expire", but on 8.0 a
  legacy `expire_logs_days` still expires logs. Also read
  `@@expire_logs_days` with a fixed statement (absent on 8.4: treat as 0);
  0 and 0 is "never expire", a non-zero days value is checked against the
  7 days; unreadable is `unchecked`.
- **SHOULD-FIX 7 (principle 2), `cdc_setup_probe.rs:253-265`.** An Agent
  query that runs but returns no Agent row becomes `NotRunning` ("SQL
  Server Agent is not running"). No row means not listed (Express, some
  platforms), not stopped: make it `unchecked` with a message that says so;
  keep `missing` for a row whose status is not `Running`.
- **SHOULD-FIX 8, `cdc_setup_check.rs:335`.** The MySQL `GRANT` command
  hard-codes the host `'%'`, so it can grant another account or fail on
  MySQL 8. Read the account with the fixed statement `SELECT
  CURRENT_USER()` and quote its user and host parts in the command.
- **SHOULD-FIX 9, `routes/connectors.rs` (`change_capture_setup_checks`).**
  The `?` on `selected_tables` runs before `record_test_outcome`, so a store
  error turns a passed test into a 500 and is not recorded, against "checks
  never change the outcome". Log the error (classified) and return no
  checks.
- **SHOULD-FIX 10, the gates (S4-h).** (a) `cdc_gate.py:889-890` "still 1
  after more ticks" cannot fail under the 15-minute dedupe: remove it, and
  instead assert the listed instance's text names the connector, the worst
  table and the limit. (b) `g4_mysql_test.py` `missing_case`: S4-h asks for
  a login without `REPLICATION CLIENT` only; grant it `REPLICATION SLAVE`
  so the gate proves each grant is checked. (c) `cdc_gate.py:792`: assert
  the gate source's list of setting names per driver, so a dropped check
  fails the gate.

NITs (fix if cheap, else note in the handoff):
- `has_replication_grants` matches substrings of the whole line; parse only
  the privilege list between `GRANT ` and ` ON `.
- MySQL commands: `SET PERSIST` instead of `SET GLOBAL` (8.0+), so the fix
  survives a restart.
- SQL Server commands: prefix `sp_cdc_enable_db`/`sp_cdc_enable_table` with
  `USE [db];` (`]` doubled); compare table names case-insensitively.
- `cdc_setup_probe.rs`: log a classified `debug` line (no server text) when
  a probe statement fails, so an operator can tell why a check is
  `unchecked`.
- The module doc's "tried again" (`capture_lag.rs:20-22`) does not hold when
  both delivery and undo fail; say so, and add the rare out-of-order case
  (a concurrent post closing an alert whose delivery then fails) and "Kafka
  Connect unreachable counts as not running" to the feature page's "Limits".
- `connector-detail-page.tsx:242`: with `?tab=capture`, the panel waits on
  the ingest spec; render it without that request.

Rules for the fixes: one commit per finding or a few related ones, each
citing `Slice 4 review SHOULD-FIX n` at the fix site and in the body; push
after the Rust group, after the Python/console group and after the gates;
then a "Slice 4, round 1 fixes" handoff with the CI run on the final code
commit.

### Slice 4, round 1 fixes (developer, Sonnet, 2026-10-30)

**Commits** (after `f56f9b3d`), each citing `Slice 4 review SHOULD-FIX n` at the fix site and in the body:
SF1 `27634018` (doc: the number is an upper bound); SF2 `c4679c5a` (a future source time stored as now, warn,
route test); SF3 `4a2dde20` (the resume sets `caught_up_at = GREATEST(caught_up_at, now())` in its
transaction, store test); SF6, 7, 8 `32f888e2` (`@@expire_logs_days` read, absent = 0, with seconds taking
precedence when non-zero; an Agent query with no row is `unchecked` through a new `NotListed` state; the
`GRANT` names the account of `SELECT CURRENT_USER()`); SF9 `819a7909` (a store error reading the tables is
logged and answers no checks); SF4 (Python) and SF5 (console) and the gates SF10 `9bcc546f`.
Pushed after the Rust group (`819a7909`), after Python/console (`736ca3e9`) and after the gates (`9bcc546f`).

**SF4 detail.** `measure_backlogs` posts a missing topic as unmeasured (left out). One table without a topic
would then have made the whole connector "could not be measured" for the sensor's run decision, so
`run_backlogs` re-measures only such a connector with `absent=0`: the run decision counts a missing topic as 0,
the other tables are not held back. Unit test `test_a_table_without_a_topic_yet_...`.

**SF10.** (a) the "still 1" assertion is gone; the gate asserts the listed `detail` names the connector, a
selected table and "over the limit of 1 min". (b) the MySQL second login has `REPLICATION SLAVE` and lacks only
`REPLICATION CLIENT`. (c) each gate asserts its source's exact setting names (`Source.expected_settings`).

**NITs done:** privilege list parsed between `GRANT ` and ` ON` (a table named like a privilege no longer
matches); `SET PERSIST`; SQL Server table names compared case-insensitively; the "tried again" doc and the
feature page Limits (resume counts from the resume, Kafka Connect unreachable = not running, two-fault
ordering, legacy `expire_logs_days`). **NITs not done:** `USE [db];` before the SQL Server enable commands;
a classified `debug` line when a probe statement fails; rendering the capture tab for `?tab=capture` without
waiting on the ingest spec.

**CI on `9bcc546f`** (run 41019425163): all 41 checks `pass`, `Rust · test (stable)` 8,649 passed 0 failed
(41m56s), clippy and fmt pass. Gates read in the job logs: `g4-cdc` (12m57s) lag after the first copy both
`up_to_date`, setup checks ok / `publication` missing alone, lag alert line "lag 64s, 1 instance
('Connector g4-demo-cdc (conn-g4-demo-cdc-mvunrev0): change capture of g4_cdc.orders_history is 1 min behind the
source, o...'); started again: both up_to_date, alert closed"; `g4-mysql` (7m47s) 5 checks ok, `replication
privileges` missing alone with `REPLICATION SLAVE` granted; `g4-mssql` (7m20s) `SQL Server Agent` `unchecked`,
`CDC on table dbo.orders_plain` missing alone. Locally: `cargo fmt --check` clean, `test_cdc_apply.py` 56
passed, `test_cdc_gate.py` 10, `bun test src/features/connectors` 428 passed, `tsc` clean, lint 0 errors. Not
verified locally: every Rust change and the gates (CI only).

### Slice 4, review round 2 (planner, 2026-10-30, code `9bcc546f`): closed

Checked by the planner: CI run 41019425163 on `9bcc546f` success (the three
change-capture gates among the jobs). Fix commits read against round 1:
SHOULD-FIX 1 (the doc says upper bound); 2 (a future source time stored as
now with a `warn`, route test); 3 (the resume moves `caught_up_at` in the
transaction that clears the pause, store test); 4 (a missing topic is
`None` in the lag post, 0 only for the sensor's run decision, unit test);
5 (Pause and Resume render from `pausedReason` when the state read fails,
test); 6, 7, 8 (`@@expire_logs_days` read, an empty Agent result is
`unchecked`, the `GRANT` names `CURRENT_USER()`); 9 (a store error answers
no checks); 10 (the alert text asserted, the MySQL login lacks only
`REPLICATION CLIENT`, the setting-name list asserted per gate). Each fix
cites its finding at the site.

Accepted as a known edge, not a finding: a failed read of
`@@expire_logs_days` counts as 0, because MySQL 8.4 removed the variable and
a failed read cannot be told apart from that. A server that refuses the read
for another reason and has `binlog_expire_logs_seconds = 0` reads "never
expire". Reading a global variable needs no privilege, so this is not
expected in practice. NITs left open, as the handoff says: the `USE [db];`
prefix, the probe `debug` line, and the `?tab=capture` wait.

No open `BLOCKER`. Slice 4 is closed. Next: slice 5 (MongoDB, session
lakehouse-35) merges in hunk by hunk and adds MongoDB's setup check on this
mechanism; then slice 6 (Oracle).
