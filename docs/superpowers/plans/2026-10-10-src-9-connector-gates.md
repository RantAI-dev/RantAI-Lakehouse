# `SRC-9` Test every advertised connector end to end — Implementation Plan

**Status:** ready to build once PR #101 is merged. Decisions D1–D7 are
signed (2026-10-10 and 2026-10-11). It is built on its own branch off `main`,
opened as a draft pull request from the first commit so CI proves each gate.
Written 2026-10-10 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/src-9.md`. Backlog `SRC-9`.
**Feature page:** `docs/core/features/connector-gates.md`. Decision numbers
below (D1–D7) are its rows.

**Base and branch:** `feat/phase-1-connector-reliability` at `36f2a2a`
(`SRC-6`, `SRC-7`, `SRC-8`, with `main` `a78ad62` merged in).

**Why.** Five supported connector types have no gate, the gates that exist
prove one step of four, and the upload gate never runs.

---

## 1. What exists today (anchors verified at `36f2a2a`)

| # | Spec row | Code | Verdict |
| --- | --- | --- | --- |
| F1 | Gate coverage: some types never gate-tested | Supported types: `rust/migrations/0035_connector_type.sql`, `0043_ingest_tier2_adapters.sql`, `0062_connector_type_unsupported_reason.sql` give thirteen. `ops/g6/g6_ingest_matrix_test.py` (`step_ingest_matrix`, `:599`) registers MySQL, SQL Server, PostgreSQL, object storage, REST, MongoDB, Kafka. `ops/g4/g4_test.py` covers PostgreSQL CDC. | No gate: Oracle, SFTP, MariaDB, MySQL CDC, SQL Server CDC. |
| F2 | "connect, discover, load, re-load" | g6 calls `POST …/test` for object storage only (`SRC-6`), never `POST …/discover`, and runs each connector once except the two tables `SRC-7` and `SRC-8` added. | One step of four for most types. |
| F3 | g9 exists, not in CI | `ops/g9/upload_test.py`; no job in `.github/workflows/ci.yml` and no service in `docker-compose.yml` names it. | Confirmed. |
| F4 | Gate runtime | Measured on PR #101 and on `main`, 2026-10-10: bringing the stack up takes 11m38s to 13m47s of the g6 job; eight g6 runs on `main` took 13m02s to 18m37s; with the `SRC-7` and `SRC-8` steps the job took 16m41s to 20m02s. Its limit was raised from 20 to 25 minutes in PR #101 (`f9d488f`). | One job cannot hold five more source systems inside 20 minutes. D1. |
| F5 | *Not in the spec* | `scripts/ci/detect_change_scope.sh` (from `main`) decides which jobs a pull request runs: g6 runs for a change under `rust/`, `dagster/`, `ops/`, `scripts/`, `docker-compose.yml` or `.env.example`; a documentation-only change skips it. | "Every PR touching ingestion" already has a mechanism; g9 joins it. |
| F6 | *Not in the spec* | Change capture runs one Debezium Server process per connector (`docker-compose.yml:2565`, PostgreSQL only). `routes/connectors.rs` (`debezium_properties`) renders properties for MySQL and SQL Server that nothing has ever started. | The two CDC gates each need a source with change capture switched on and a Debezium Server of their own. |
| F7 | *Not in the spec* | Nothing compares the supported types with what the gates cover. | Coverage can fall below 100% silently. D6. |

## 2. Decisions already made by the planner

- **One registry of what is gated.** `ops/gates/coverage.json`: for each
  supported type, the gate that covers it and, per step, `real` or the
  refusal the product is expected to give (D2). The gates read it and the
  coverage check reads it; neither keeps a second list.
- **The summary table is written by the gates**, from what they ran, to the
  job summary. A step that did not run is `skipped` with its reason, never
  blank and never green.
- **Jobs.** `g6-ingest` keeps the batch sources and gains MariaDB and SFTP.
  Oracle gets its own job. `g4-cdc` gains MySQL CDC and SQL Server CDC as two
  more sources if the measured time allows; otherwise they get a job of their
  own. `g9-upload` is new. All are behind the scope gate and in
  `ci-required`.
- **No product code changes** unless a gate finds a defect. A defect is
  reported to the planner and fixed in its own commit with its own finding
  id; a gate is never bent around it.

## 2a. Spec row to tasks

| Spec Target row | Tasks |
| --- | --- |
| Gate coverage, four steps, every supported type | 1, 2, 3, 4, 5, 6, 8 |
| g9 in CI on every PR touching ingestion | 7 |
| Connector gates within 20 minutes | 9 (measure), 10 (write up) |

## 3. Tasks

One task per commit. Every task that adds a compose service is proven by a
real `up` of the affected profile from a clean project (rule 8); where that
runs is decision 7.

0. **G3's start-up race (found on PR #101).** `ops/g3/g3_test.py` triggers
   `bronze_ingest_job` before the code location has finished loading and gets
   `422 PipelineNotFoundError` (PR #101, run of 2026-10-10; a re-run passed).
   `ops/g6/g6_ingest_matrix_test.py` already waits and retries once for the
   same race (`_post_ingest_run`). Give g3 the same handling, reusing the
   helper rather than copying it, and add the backlog item for it with the
   next free ID when this branch is cut (product owner, 2026-10-11).
1. **Registry and summary.** `ops/gates/coverage.json` and a small shared
   module the gates import to record a step result and to write the summary
   table. No behaviour change to any gate yet.
2. **Four steps for the types g6 already loads.** For each of PostgreSQL,
   MySQL, SQL Server, object storage, REST, MongoDB, Kafka: connect
   (`POST …/test`), discover (`POST …/discover`), load, and a second load that
   asserts the table's rows as the load mode promises. Where the product
   refuses, assert the refusal's fixed shape (`supported: false`), not a
   string that may be reworded.
3. **MariaDB.** A `mariadb-g6` service in the g6 override; the four steps.
4. **SFTP.** An SFTP server in the g6 override with a known host key, a CSV
   fixture, and the four steps. The connector's host-key fingerprint is the
   fixture's own.
5. **Oracle.** Its own profile, service and CI job (D5); the four steps;
   `ORACLE_*` secrets as `${X:?}` in the override, never a default.
6. **MySQL CDC and SQL Server CDC.** A source with change capture on, the
   properties the API renders written where a Debezium Server reads them, and
   the steps of `ops/g4` for PostgreSQL: first copy, then an insert, an update
   and a delete arriving within the budget g4 uses, then deletion of the
   connector leaving nothing behind at the source.
7. **g9 in CI.** A `g9-test-runner` service and a `g9-upload` job, under the
   scope gate with the same condition as g6, in `ci-required`.
8. **Coverage check (D6).** `ops/lint/check_connector_gate_coverage.py`:
   reads the supported types from the migrations and the registry, fails on a
   type with no gate or a gate for a type no longer supported; a job step in
   `repo-lints`; unit-tested with fixtures.
9. **Measure.** From CI runs on the final commit: each connector gate job's
   duration, three runs. If the slowest is over 20 minutes, report it; the
   target is not lowered and the job is not shortened by removing steps.
10. **Docs (planner).** `docs/plans/SRC-9-RESULT.md`, `CHANGELOG.md`,
    `docs/OPERATIONS.md` or the gates' README, the spec's status and its
    "Today" row for MariaDB.

## 4. Build limits on this machine

No docker and no gate can run here. Python lints and unit tests run for real
in `~/.cache/rantai-dagster-venv`. Everything else in this plan is proven
only by CI (decision 7).

## 5. Out of scope

- Any change to what a connector does. Discovery for more types (`SRC-13`);
  a connection test for Oracle, MongoDB, Kafka, SFTP (`SRC-5`).
- Row-filter tests (`VER-1`). Google Sheets.
- Load, soak or version-matrix testing of a source system.

## Handoff

### Tasks 0-2 (developer, first pass)

Commits on `feat/src-9-connector-gates`: `bd5b679` (task 0), `c708ae2`
(task 1), `c1f09c7` (task 2).

**Commands run here, with results.** `python3 -m py_compile` on
`ops/g3/g3_test.py`, `ops/g6/g6_ingest_matrix_test.py`,
`ops/g6/rest_stub.py`, `ops/gates/gate_summary.py` and
`ops/gates/test_gate_summary.py`: no output (ok).
`python3 -m unittest discover -s ops/gates -p 'test_*.py'`: `Ran 27 tests ... OK`.
`python3 ops/lint/check_intra_package_imports.py`, `check_bare_iceberg_count.py`,
`check_compose_init_readiness.py`: all pass. `yaml.safe_load` of
`.github/workflows/ci.yml` and `ops/g6/docker-compose.g6.override.yml`: ok.
`docker compose -f docker-compose.yml -f ops/g6/docker-compose.g6.override.yml
--profile g6 --profile dagster config` (dummy values for the `:?` variables):
resolves, and shows the bind mount of `ops/gates` at `/opt/gates` read-only and
`PYTHONPATH=/opt/gates`. A throwaway script (not committed) drove
`_check_connect` / `_check_discover` against a fake session to catch typos.
No cargo, no bun.

**Task 0.** g3 now waits for `bronze_ingest_job` to be listed after the
webserver is up (`step_wait_for_services`) and retries the trigger once on
`422 PipelineNotFoundError` (`_post_trigger`). Sharing: not possible without a
compose change. Each runner mounts only its own `ops/gN` as `/work` (g3 also
mounts one extra file, `bronze_catalog.py`, by an explicit volume line), so g3
cannot import g6's helper, and a shared module needs a mount in
`g3-maintenance-test-runner` in `docker-compose.yml`. g3 therefore carries a
twin of g6's `_ingest_job_is_loaded` / `_post_ingest_run`, with a comment
naming g6's functions. Other gates with the same exposure: g3a
(`step_trigger_run_via_lakehouse_api` triggers `bronze_ingest_job` after only
listing it, no wait and no retry) and g7 (`step_trigger_and_wait_for_real_row_
materializations`, a plain trigger); g4 already waits for
`replication_slot_check_job` before its launch (no retry). Not fixed, as asked.
The backlog item for the race (plan task 0, "next free ID") is not added: the
backlog is the planner's file.

**Task 1.** `ops/gates/coverage.json` (13 types), `ops/gates/gate_summary.py`
(`Registry`, `Recorder`, `extract_summary`), tests in
`ops/gates/test_gate_summary.py`; run them with
`python3 -m unittest discover -s ops/gates -p 'test_*.py'` from the repository
root (there is no test home under `ops/lint`). Sharing for g6: a read-only
mount `./ops/gates:/opt/gates:ro`, `PYTHONPATH=/opt/gates` and
`PYTHONDONTWRITEBYTECODE=1`, added in `ops/g6/docker-compose.g6.override.yml`
only (not `docker-compose.yml`). This is a compose-file change the brief did
not list; the module cannot reach the container otherwise. Job summary: the
container cannot write the host `GITHUB_STEP_SUMMARY`, so the gate prints the
Markdown between the lines `GATE-SUMMARY-BEGIN` / `GATE-SUMMARY-END` (plain
text, not `::x::`, which GitHub reads as workflow commands), and `ci.yml`'s
g6 job now runs the gate as `... run --rm -T g6-test-runner 2>&1 | tee
$RUNNER_TEMP/g6-run.log` (`shell: bash` for `pipefail`) and a new
`if: always()` step copies the marked lines into `$GITHUB_STEP_SUMMARY` with
`awk`. `emit()` also appends to `GITHUB_STEP_SUMMARY` when that variable is
set in the process (a host run).

Supported types derived from `0035` (ten `supported = true`: PostgreSQL,
PostgreSQL CDC, MySQL, MySQL CDC, MariaDB, SQL Server, SQL Server CDC, Object
storage, REST API, Google Sheets), `0043` (+ Kafka, MongoDB, SFTP, Oracle) and
`0062` (- Google Sheets): thirteen, matching the plan. No other migration
touches `connector_type` (`0036` only mentions the number 0035 in a comment).

Registry values (connect / discover / load / reload):

| Type | Gate | Values |
| --- | --- | --- |
| PostgreSQL | g6 | real / real / real / real |
| PostgreSQL CDC | g4 | missing / missing / real / real |
| MySQL | g6 | real / real / real / real |
| MySQL CDC | null | missing x4 |
| MariaDB | null | missing x4 |
| SQL Server | g6 | real / real / real / real |
| SQL Server CDC | null | missing x4 |
| Object storage | g6 | real / refused (SRC-13) / real / real |
| REST API | g6 | real / refused (SRC-13) / real / real |
| Oracle | null | missing x4 |
| MongoDB | g6 | refused (SRC-5) / refused (SRC-13) / real / real |
| Kafka | g6 | refused (SRC-5) / refused (SRC-13) / real / real |
| SFTP | null | missing x4 |

The decisions are read from `connector_probe::probe` and
`connector_discover::discover`. `notes` in the file record where the product
behaviour for a `missing` cell is already known (Oracle and SFTP connect:
`supported: false`, SRC-5).

**Task 2.** `ops/g6/g6_ingest_matrix_test.py`: `_check_connect`,
`_check_discover`, `_step_reload`, `_produce_kafka_reload_batch`, and the
recording of the existing load checks. A failed new step is recorded and the
gate goes on (one CI run shows every broken cell); `main` raises one
`G6Failure` at the end whose `[g6] FAILED:` line lists `<type> <step>: <detail>`
for each, and the table prints in a `finally` on every path. Existing
assertions: none weakened or removed. Changed by wrapping only: the run-wait
loop and the Bronze verification loop (same asserts, now inside a `try` that
records the load cell as `fail` and re-raises), the object-storage test call
(same call, recorded as the `connect` cell), `connector_ids` now filled for
every connector. `reload` for a default-mode connector: `loadMode` absent means
`replace` (`LoadPlan.from_source_object`), so the second run must leave the same
count; the count is read until it matches, then again 6 s later, because an
older snapshot has the same count a `replace` should keep. Kafka (append):
the gate produces 5 new messages after the first run's offsets are committed,
then computes, at the broker, (end offset - the group's committed offset) summed
over partitions, which includes any message the first run's producer sent after
the consumer's last poll; the second run must add exactly that many. If the
group has no committed offset the cell is `skipped` with the reason. The
second runs are all started, then all waited for.

`ops/g6/rest_stub.py` now answers `GET /` with 200: `connector_probe::
probe_rest` sends one GET to the connector's `baseUrl` itself and the stub
answered 404, so a "real" REST connect could never pass.

**Time estimate (not measured).** connect and discover: 14 short calls, made
while the first runs are in flight, about 0 to 30 s. reload: the seven second
runs start together; the slowest is Kafka, whose consumer polls for its whole
60 s window, so about 70 to 80 s from launch, plus the count wait and one 6 s
stability pause, plus Kafka offset/produce setup of a few seconds. Estimate
1.5 to 2 minutes added, inside the 3-minute line; the 25-minute timeout is
untouched.

**Plan/code mismatches found.**
1. The REST stub returned 404 for `/` (fixed above).
2. Oracle discovery answers HTTP 422 (`DiscoverError::Blocked` for
   `SqlDriver::Oracle`), not `supported: false`. The registry marks Oracle
   discover `missing` with a note; task 5 must either assert the 422 or the
   planner changes the product to a `supported: false` answer.
3. Plan task 0 and task 1 say a shared module; without a compose change that
   is only possible for g6, by the override mount above. g3 stays a twin.
4. Plan section 2 says "the gates read it and the coverage check reads it":
   g4 does not use the module yet; its registry cells (`load`/`reload` real)
   are not recorded by any run today, so the PostgreSQL CDC row shows
   `skipped` ("covered by gate g4, which did not run here") in g6's table.

**Not verified.** Everything about the gates: no gate, docker run or CI ran. In
particular: (a) the `up`/`run` of the g6 profile with the new mount and `-T`/
`tee`; (b) that MySQL, SQL Server and PostgreSQL discovery return the fixture
tables with those exact names and column names (MySQL 8's
`information_schema` columns may decode differently through sqlx; if
discover fails for MySQL, that is a product finding); (c) that the connection
test passes for the seeded `conn-pg-lakehouse` (credential
`env:CONNECTOR_PG_PASSWORD` resolved by the API); (d) that a second `replace`
run leaves the same count for every adapter and that Kafka's broker-side
arithmetic gives the number the table grows by (`KafkaConsumer.committed` and
`end_offsets` with only a `group_id`); (e) the `awk` extraction in the new CI
step. Expected risks in CI, most likely first: Kafka reload arithmetic, MySQL
discover decoding, REST connect, the pg connect.


### Second pass (developer): F-a, F-b, F-c, tasks 3 and 4

Commits after `f5cef8a`: `fe76d3d` (F-a), `a0fbc77` (F-b), `1c5b79d` (F-c),
`08adff1` (task 3), `987dd65` (task 4).

**F-a, finding G1 (MySQL discovery 422 "connection failed").** Found by
reading, not proven: `connector_probe::probe_mysql` and
`connector_discover::discover_dial` build their options with the same
`mysql_connect_options` (pinned address, `Preferred` TLS, port, database,
user, password), so the connection is not the difference. The difference is
the query: discovery decodes `information_schema.columns` rows into `String`,
and `MySQL` 8 reports those columns with a binary collation, which `sqlx`
refuses (`ColumnDecode`); `classify_sqlx_error` has no arm for it and answers
"connection failed". `MYSQL_DISCOVER_QUERY` now casts the three columns to
`CHAR CHARACTER SET utf8mb4` (a `const`, one `?`). `From<DiscoverError>` now
logs `sqlx_kind` (the variant name, a fixed string) and the class; never the
error text, which for a database error carries the server's message. The
module comment that said MySQL/SQL Server discovery has no live test is
corrected. Confidence: moderate; CI (g6) decides, and the new log line names
the cause if it is another.

**F-b, finding G2.** `discover` answers `supported: false` with a fixed reason
(`ORACLE_DISCOVER_REASON`, names `SRC-13`) before any SSRF check or secret
resolution. No existing test asserted the 422 (searched `rust/crates/*/tests`
and `src`); a new unit test, `oracle_discovery_is_an_honest_unsupported_answer`,
asserts the new answer. Mismatch: the brief asked for Oracle's `discover` cell
to be `refused`; `Registry.load` rejects any non-`missing` step for a type with
no gate (`ops/gates/gate_summary.py`), and weakening that is not mine to do, so
the cell stays `missing` with a note saying `refused` is expected once the
Oracle gate exists (task 5).

**F-c.** `ops/g3a/g3a_test.py` and `ops/g7/g7_pipeline_test.py` carry twins of
g3's `_dagster_job_is_loaded` / `_post_trigger`; g3a also waits for its job in
`step_wait_for_services`. No assertion changed. g7 is not run by any CI job or
compose service today.

**Task 3.** `mariadb-g6` (`mariadb:11.4`, the LTS series) in the override,
`MARIADB_G6_ROOT_PASSWORD` and `CONNECTOR_MARIADB_PASSWORD` (`:?`), TCP
healthcheck, wire driver `mysql`. **Task 4.** `sftp-g6`
(`atmoz/sftp:alpine@sha256:7e43...813f`), `sftp-g6-hostkey-init`,
`SFTP_G6_PASSWORD`; the gate derives the pinned fingerprint from the fixture's
public key and uploads a 3-row CSV as the user.

**Time estimate (not measured).** g6 was 19m51s. Added: two image pulls
(about 110 MB MariaDB, 9 MB SFTP) inside the parallel `up`, 20 to 40 s; two
more connectors whose first and second runs are launched with the others,
about 30 to 60 s each stage. Estimate +1 to 2.5 minutes, so 21 to 22.5 minutes,
under the 23-minute line; the timeout is untouched.

**Commands run, with results.** `cargo fmt --check`: ok. `cargo clippy
--workspace --all-targets --all-features --locked -- -D warnings` (shared
cache dir, 2 jobs): finished clean, 2m33s. `py_compile` on `ops/g3a`, `ops/g7`
and `ops/g6` files: ok. `python3 -m unittest discover -s ops/gates -p
'test_*.py'`: 27 tests OK. `check_intra_package_imports.py`,
`check_bare_iceberg_count.py`, `check_compose_init_readiness.py`: pass.
`yaml.safe_load` of `ci.yml` and the override: ok. `docker compose -p check
-f docker-compose.yml -f ops/g6/docker-compose.g6.override.yml --profile g6
--profile dagster config --quiet` with a dummy env file: ok, and it fails
closed (names the missing variable) without the env file.

**Not verified.** No cargo test (the new Rust unit test and the existing
query-text test have not run); nothing about the containers: that `atmoz/sftp`
runs `/etc/sftp.d` scripts before sshd and accepts the installed key, that
`mariadb-admin` ping works as the healthcheck, that sqlx and pymysql
authenticate to MariaDB 11.4, that the `CAST` fixes the MySQL decode, the Kafka
reload arithmetic from the first pass, the time estimate.

**Expect from CI.** MySQL discover passes if the cast was the cause, otherwise
the API log now names the sqlx variant. MariaDB: four passes. SFTP: connect and
discover `refused-as-expected` (discover's reason today reads "unknown adapter
\"sftp\"", not a statement that discovery is unsupported; reason text is not
asserted), load and reload pass. Oracle stays `missing`.

### Third pass (developer): tasks 7, 5, 6, 8

Commits after `9c1846f`: `350266a` (task 7), `5ccaab8` (task 5), `46139f4`
(task 6), `ee359cc` (task 8).

**Task 6 finding (no MySQL CDC or SQL Server CDC gate built).** In g4 one
static `debezium-server` (`docker-compose.yml`, heredoc of
`application.properties`, PostgreSQL only) captures `p5_cdc.orders`; nothing
renders it from the API. `GET /api/connectors/{id}/debezium-properties`
(`routes/connectors.rs`, `debezium_properties`; it is a GET, not a POST)
calls `render_debezium_properties_template`, whose non-Mongo branch
(`lakehouse-store/src/cdc.rs:664-673`) is the PostgreSQL shape with only
`connector.class` swapped (`connectors.rs:1888`, `debezium_connector_class`).
Read from the Debezium 3.6.3.Final sources (the version of the pinned
`debezium-server-iceberg:1.1.1.Final`, from its release notes): MySQL's
`database.server.id` is `.required()` (`BinlogConnectorConfig.SERVER_ID`) and
is never rendered (`CdcDial.server_id`, `ingest_spec.rs:117`, is read nowhere
in `cdc.rs` or `connectors.rs`); SQL Server's `database.names` is validated
non-empty (`SqlServerConnectorConfig.DATABASE_NAMES`) and the template renders
`database.dbname`, which that connector does not define. Both also receive
the PostgreSQL-only `plugin.name=pgoutput`, `slot.name={slug}_slot` (derived
from the connector id, not from `dial.slotName`), `publication.name`,
`publication.autocreate.mode`. A Debezium Server started from either fails
configuration validation. Also: `DELETE /api/connectors/{id}` and
`ops/debezium/deprovision_connector.sh` clean up PostgreSQL slots only, so
"deletion leaves nothing behind" has no MySQL or SQL Server counterpart.
Decision for the product owner: build the templates (a separate item), or
`supported = false` with a reason as `SRC-6` did for Google Sheets. Both
types stay `missing` in `coverage.json` with the gaps in their notes, waived
until `SRC-4` (the item that already lists "live change capture ... each
gate-tested"), which the planner may re-point. Not verified: `database.encrypt`
for SQL Server (JDBC encryption defaults) and the MySQL schema-history
storage choice, since the first blocker comes before them.

**Task 7.** `g9-test-runner` (profile `dagster`, `ops/g9` and `ops/fixtures`
read-only under `/work`) and job `g9-upload`, 25 minutes, the scope-gate
`if:` of `g6-ingest`, CI-only `.env`, teardown. Two things the gate needs
that no other job gives: `UPLOAD_S3_ACCESS_KEY`/`UPLOAD_S3_SECRET_KEY` for
`lakehouse-api` (no default; unset, uploads answer 503), set in the CI `.env`
to RustFS's own default root key (`rustfsadmin`, RustFS has no other
identity, see the `CONNECTOR_S3_*` note), and `clickhouse-iceberg-init` in the
`up` list (the API's `ICEBERG_QUERY_DB`; nothing depends on it).
`ci-required`: `needs`, `G9_UPLOAD_RESULT`, `required_gate.sh`, its self-test
(27 -> 28 cases), `docs/CI.md`. `ops/g9/**` is `stack` in
`detect_change_scope.sh` (confirmed; test case added, 33 pass).

**Task 5.** `ops/g6/docker-compose.g6-oracle.override.yml` (own profile
`g6-oracle`, own runner `g6-oracle-test-runner`, does not combine with the g6
override), `ops/g6/g6_oracle_test.py` (imports the matrix gate's helpers),
job `g6-oracle`. Image `gvenzl/oracle-free:23.26.2-slim-faststart@sha256:5e340d72c5cd93b5ce8bab6c128b8f37a5ec485d5d9d98dc2cdc5353fe640f96`
(index digest, from the Docker Hub registry API today). Healthcheck
`/opt/oracle/healthcheck.sh`, verified by reading it out of the image's layer:
exit 0 only when `FREEPDB1` and the container database are `READ WRITE` and
the init flag files are gone. The gate also waits for a real thin-mode login.
Licence: the database is under Oracle's Free Use Terms and Conditions, no
interactive acceptance variable exists; that is decision 5's licence question
for the product owner. `oracledb==2.5.1` is already in the dagster image the
runner is built from. `coverage.json`: Oracle -> `g6-oracle`, connect and
discover `refused`, load and reload `real`; `test_gate_summary` removed
Oracle's gate inside the test instead of relying on it being gate-less (same
assertion).

*Probable defect, not proven:* `adapters/oracle.py` passes the dial's
`database` (labelled "Database (service name)" in the console) to SQLAlchemy as
the URL path, which `cx_oracle.py:1347-1349` hands to `makedsn(sid=...)`, so
`FREEPDB1` is sent as a SID, not a service name. Expect `ORA-12505` on load.
The fix would be a `service_name` query argument in the adapter (Python, not
done here). The gate was not bent around it.

**Task 6 (g4).** `g4_test.py` records `PostgreSQL CDC` through `gate_summary`:
connect and discover (the demo source registered as a `PostgreSQL CDC`
connector with its real slot and publication, `POST .../test`,
`POST .../discover?schema=p5_cdc`), load (new: the snapshot holds the 5 seeded
rows) and reload (the three existing assertions, only wrapped to record).
Needs `ops/g4/docker-compose.g4.override.yml` (a `file:` credential volume and
`CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`, gate-only) on every compose call of the
`g4-cdc` job, and the `ops/gates` mount in `g4-test-runner`. The job copies
the table to its summary.

**Task 8.** `ops/lint/check_connector_gate_coverage.py` and 23 unit tests;
`Registry` validates and returns a `waiver`. A step in `repo-lints` runs it
and the unit tests of `ops/lint` and `ops/gates`. On the real tree: 13
supported types, 11 covered, 2 waived (MySQL CDC, SQL Server CDC, until
`SRC-4`), exit 0.

**g6 time.** Nothing here touches the `g6-ingest` job (the diff of `ci.yml`
has no hunk inside it).

**Commands run, with results.** `py_compile` on `ops/g4/g4_test.py`,
`ops/g6/g6_oracle_test.py`, `ops/g6/g6_ingest_matrix_test.py`,
`ops/g9/upload_test.py`, the lint and its test, `ops/gates/gate_summary.py`:
ok. `python3 -m unittest discover -s ops/gates -p 'test_*.py'`: 29 tests OK.
`python3 -m unittest discover -s ops/lint -p 'test_*.py'`: 23 tests OK.
`check_intra_package_imports.py`, `check_bare_iceberg_count.py`,
`check_compose_init_readiness.py`, `check_connector_gate_coverage.py`: pass.
`bash scripts/ci/tests/test_required_gate.sh`: 28 passed;
`test_detect_change_scope.sh`: 33 passed. `yaml.safe_load` of `ci.yml` and every
compose file touched: ok. `docker compose -p check [-f override] --profile ...
config --quiet` with a dummy env file for: base + `dagster` (g9), g6 + `g6` +
`dagster`, g6-oracle + `g6-oracle` + `dagster`, g4 + `dagster`, and
`--profile '*'` on the base file: all ok; the Oracle file fails closed naming
`ORACLE_G6_PASSWORD` when the env lacks it. No cargo, no bun, nothing started.

**Not verified (all of it needs CI).** That any container starts: `g9-upload`
(upload credential accepted by RustFS, fixtures at `/work/fixtures`, the
table readable through `icecat_api`); `g6-oracle` (image pulls and opens in
time on a runner, the healthcheck, the adapter dial, the SID defect above, the
job's duration); g4 (connect and discover answer for real for a `cdc`
connector, the file credential resolves, the snapshot holds exactly 5 rows when
first visible, the `-f` pair and `-T`/`tee` in the g4 job); the `awk` summary
steps. The Debezium findings are from sources, not from running a server.

**Expect from CI.** `g9-upload`: pass, if the upload credential works; most
likely failures are the 503 "Upload storage" (credential) or the table not
visible through `icecat_api`. `g6-oracle`: load fails with an Oracle
connection error if the SID defect is real; connect and discover show
`refused-as-expected`. `g4-cdc`: connect and discover are the new risk; a
failure there is recorded, the Debezium steps still run, and the verdict names
the cell. `repo-lints`: pass.

## Review

*(planner)*
