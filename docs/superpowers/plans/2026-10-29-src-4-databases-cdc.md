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

Amended in detail before dispatch. Fixtures with the source settings on
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

(Further entries follow, one per slice.)

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
