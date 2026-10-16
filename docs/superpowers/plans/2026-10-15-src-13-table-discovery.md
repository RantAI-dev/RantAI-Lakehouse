# `SRC-13` Table discovery for every source type — Implementation Plan

**Status:** ready to build once `SRC-11` is green on #144 (the two share the
branch and the worktree; building them at the same time would make their
commits collide). The product owner signed the feature page's decisions on
2026-10-14 ("setuju semua"). Written 2026-10-15 by the planner (Claude Opus)
for developer agents, under the role split in `AGENTS.md`.

**Base and branch:** the Phase 1 branch `feat/phase-1-rest` (draft PR #144).
**Spec:** `docs/core/specs/src-13.md`. **Feature page:**
`docs/core/features/table-discovery-every-source.md`; D1 to D12 below are
its decisions 1 to 12.

---

## 1. What exists today (read on 2026-10-14 for the feature page; the developer re-verifies every anchor, line numbers move)

| # | Spec row | Code | Gap |
| --- | --- | --- | --- |
| F1 | Coverage | `lakehouse-api/src/connector_discover.rs` `discover`: real lists for `sql`/`cdc` with PostgreSQL, MySQL/MariaDB, SQL Server, one schema per call, schema typed (`?schema=`). Oracle refused with `ORACLE_DISCOVER_REASON` (claims the orchestrator lists Oracle; nothing does: `adapters/sql.py` `LISTABLE_DRIVERS` leaves Oracle out). Files and REST: "not implemented in this build". MongoDB, Kafka, SFTP fall to the catch-all `unknown adapter "…"`. Every refusal is HTTP 200 `supported:false`. | D1 to D5, D7, D10 |
| F2 | Coverage (console) | `connector-ingest-panel.tsx`: `canDiscover` only for `sql`/`cdc`; an Oracle connector sees "Find tables", gets the refusal, and has no "add by name" field; Kafka shows no table list (topic typed in the dial form); REST "add by name" names a target but every object loads every endpoint (`adapters/rest.py` `build_source` ignores `source_objects`). | D3, D4, D5 |
| F3 | Column preview | SQL discovery fetches columns and types; the console shows only a count. No preview for the other types; their columns are learnt after the first load (`schema_observer` `after_load`). The API already parses CSV headers and Parquet schemas for uploads (`upload_parse::preview`, `upload_parquet::inspect`). | D6 |
| F4 | Large sources | No limit, paging or search on the server; all columns of all tables in one query, inside the 5-second `DIAL_TIMEOUT`; the console draws every row (a client-side filter appears above 8 rows). Not measured. | D9 |
| F5 | Where it can run | The API has `sqlx` (PostgreSQL, MySQL), `tiberius`, `object_store` (S3) and `reqwest`; no MongoDB, Kafka, SSH or Oracle client. The orchestrator has `pymongo`, `kafka-python`, `paramiko`, `oracledb`, `s3fs` and the adapters' own connection code. There is no synchronous API-to-orchestrator call: runs are launched and polled; the orchestrator posts results back on `ingest:read` service routes (`schema-observations`, and since `SRC-11` the run reports). | D1 |
| F6 | Gates | `ops/gates/coverage.json`: `discover` is `real` for PostgreSQL, PostgreSQL CDC, MySQL, MariaDB, SQL Server and `refused` for the other six; `_check_discover` in `ops/g6/g6_ingest_matrix_test.py` does not check the refusal's words. | D12 |

## 2. Decisions as built (from the feature page; the planner's notes in italics)

- **D1. Where the listing runs.** API: SQL sources (as today), object
  storage (S3 listing), REST (the declared endpoints; no network).
  Orchestrator: MongoDB, Kafka, SFTP, Oracle. *Also the orchestrator: the
  column preview of REST (first page, through the adapter's own pagination
  and records path, so the preview is what a load would read), MongoDB,
  Kafka and SFTP files.* An orchestrator listing is a short Dagster job
  (`discover_job`) launched by the API; its result is posted back on a new
  service route (`ingest:read`, `require_orchestrator`, closed fields, as
  `SRC-11`'s run reports) and stored; the console polls. One stored result
  per connector and kind (latest wins), so nothing grows without bound.
- **D2. Files by glob.** The user types a glob (default: everything under
  the connector's prefix); the list shows each matching key with size and
  last change; at most 10,000 keys, and the answer says when it was cut.
  Each ticked file is its own table, as a typed name is today.
- **D3. REST.** The list is the dial's `endpoints`; a source object names
  one endpoint (its path) and loads that endpoint only. *Objects saved
  before this name a target, not an endpoint: they keep loading every
  endpoint until re-picked, and the console says so on such a row.*
- **D4. Kafka.** The broker's topics, internal ones (`__…`) hidden; picking
  one writes the dial's `topic` (one topic per connector).
- **D5. MongoDB, SFTP, Oracle.** MongoDB: the collections of the dial's
  database. SFTP: files under the dial's path, by glob as D2. Oracle: the
  schemas the user can read, then that schema's tables with columns and
  primary keys, as the other SQL sources; and the "add by name" field.
- **D6. Column preview.** SQL and Oracle: declared columns and types. CSV:
  header plus the first 1,000 rows; Parquet: its schema. MongoDB: 100
  documents. Kafka: the last 100 messages, read without committing an
  offset and with a group id the connector never uses. REST: the first page.
  An inferred type is marked "inferred from a sample".
- **D7. Schemas** are listed for every SQL type, picked from a list;
  typing stays possible.
- **D8. Views** are not listed (base tables only, as the loader lists).
- **D9. 10,000 tables in 2 minutes, searchable.** Names first, columns when
  a table is opened (a second call per table). The listing call's own
  limit is 120 s (not the 5-second test limit). The console searches on the
  server for more than a page and draws only the rows in view. Measured by
  a gate on a PostgreSQL schema of 10,000 tables; the time is printed and
  the step fails above 120 s.
- **D10. Errors.** Fixed texts per class: unreachable, sign-in refused, no
  permission at the source, timed out, too many objects, not supported for
  this type. The wrong "unknown adapter" text goes; the Oracle text no
  longer claims the orchestrator lists it.
- **D11.** `connector:manage`, own tenant, as today.
- **D12. Gates.** Each type's `discover` step becomes a real listing with a
  known object and its columns; `coverage.json` follows; the 10,000-table
  measurement runs in a job of its own.

## 3. Tasks

One task per commit; cite `SRC-13` and the decision at each site.

1. **Store and wire contract (D1, D9, D10).** A table for orchestrator
   discovery results (connector, kind: `list` or `preview`, object, state,
   error class from a closed list, the result as JSON with a size cap,
   started/ended), migration with the next free number and a why-header.
   The response shape shared by every type: `{state, objects:[{name, kind,
   size?, modifiedAt?}], truncated, reason?}` for a listing and `{state,
   columns:[{name, typeName, inferred}], primaryKey?}` for a preview. Store
   tests.
2. **API listings (D2, D3, D7, D8, D9, D10).** SQL: schema list; names-first
   listing (base tables only); per-table columns and key on demand; the
   120-second listing limit. Object storage: S3 listing with prefix and
   glob, 10,000 cap, CSV and Parquet preview reusing the upload parsers on
   a bounded range read. REST: the endpoint list from the dial. The routes
   (listing, preview, poll) in `POLICY_TABLE` and `tests/route_auth.rs`;
   the old `?schema=` call keeps working until the console moves (then the
   consumer moves, no mock). Route tests per type and per error class.
3. **API launches orchestrator listings (D1, D4, D5).** MongoDB, Kafka,
   SFTP, Oracle (and REST/MongoDB/Kafka/SFTP previews): the route launches
   `discover_job` with the connector id, kind and object, returns
   `{state: "running"}`; the result route accepts the orchestrator's post;
   the poll returns it. A launch failure is a fixed text. Tests with the
   Dagster mock.
4. **Orchestrator `discover_job` (D1, D4, D5, D6, D10).** Reuses each
   adapter's connection code, credentials and address checks; never loads;
   bounded (10,000 names, 100 samples, 1,000 rows); classifies failures with
   `SRC-11`'s classifier mapped to D10's classes; posts the result with
   `timeout=` and `raise_for_status()`. Kafka reads without committing.
   Unit tests, no network.
5. **REST loads one endpoint per object (D3).** `adapters/rest.py`: an
   object that names a declared endpoint loads that endpoint only; an older
   object keeps today's behaviour. Unit tests.
6. **Kafka topic choice (D4).** Picking a topic updates the dial's `topic`
   through the existing spec save (so `SEC-14`'s re-point rule still
   applies if it counts as a target change — check and say which).
7. **Console (D2 to D10).** One "Find" flow for every type: schema list for
   SQL, glob for files and SFTP, topic list for Kafka, endpoints for REST,
   collections for MongoDB; a progress state while an orchestrator listing
   runs; search; windowed rendering for long lists (no new dependency unless
   one is already in `package.json`); the columns panel with "inferred from
   a sample"; Oracle's "add by name"; the REST row note of D3. Contracts
   mirror serde. Component tests.
8. **Gates (D12).** `discover` steps real for all eleven types in the
   existing g6 groups (each step checks a known object and its columns);
   `coverage.json`; the Oracle gate; a new job that creates 10,000 tables in
   a PostgreSQL schema, lists them, searches one, and prints the seconds
   (fail above 120). `ci-required`, `required_gate.sh` and its self-test,
   `docs/CI.md`. No existing job may grow past 20 minutes; if one would,
   the step moves to the new job.
9. **Docs (planner).** `docs/plans/SRC-13-RESULT.md`, `CHANGELOG.md`,
   `BACKLOG.md`, the feature page's status.

## 4. Build limits on this machine

As `SRC-11`'s plan: no `cargo test`/`build`/`nextest`/`clean`, no
containers; no compiling cargo command under 12 GB free; shared target
`/home/hv/.cache/lakehouse-src6-target` with `CARGO_BUILD_JOBS=2`,
`CARGO_INCREMENTAL=0`, sources touched before the workspace clippy, run
detached. The console's whole test suite is killed by the machine's memory:
run it per directory and say so. Rust tests and gates are proven by CI on
#144.

## 5. Out of scope

- One table loaded from many files matched by a glob.
- More than one Kafka topic per connector.
- Connection tests for MongoDB, Kafka, SFTP and Oracle.
- MySQL CDC, SQL Server CDC, Google Sheets.

## Handoff

### Tasks 1 to 4 (developer)

Branch `feat/phase-1-src-13`, base `1c3efd25`. Commits: `42f09344` (task 1, store and contract), `07d0f5f1` (task 2, API listings), `307f1e38` (task 3, orchestrator launch and result route), `7a0ee689` (task 4, `discover_job`). Migration `0075_connector_discovery.sql`.

**Commands run (outcomes).**
- `cargo clippy --locked --offline -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings` (shared target, jobs 2): finished clean on the final Rust tree (after fixing 4 rustdoc list-indent errors and a `Debug` bound in a test it found).
- `rustfmt --edition 2024 --check` on every touched Rust file: clean.
- `(cd dagster && python -m pytest dispar_orchestrate -q -m "not loader")`: 968 passed (53 of them `test_discover.py`).
- `check_intra_package_imports.py`, `check_bare_iceberg_count.py`: OK.

**Not run.** `cargo test` (forbidden here): every Rust test, including the `sqlx::test` and route tests, is unproven locally; CI is the proof. Test gates, console, compose: not touched.

**Tests added.** `lakehouse-store/tests/connector_discovery.rs` (9), unit tests in `connector_listing.rs` (globs, LIKE escape, type inference, class mapping, dispatch, live PostgreSQL schemas/tables/columns), `routes/connector_discovery.rs` (bodies, kinds, result contradictions), `routes/mod.rs` (130 s timeout), `connector_discover.rs` (views gone, fixed sentence), `tests/connector_discovery_routes.rs` (6), `tests/connector_discovery_launch.rs` (3, wiremock Dagster), `route_auth.rs` (+1, both directions), `dagster/.../test_discover.py`.

**Wire shapes (the console and gates build on these).**
- `POST /api/connectors/{id}/discovery/{kind}`, `kind` = `list` | `preview`; `GET` same path reads the latest answer (404 if none; a `running` request older than 300 s reads as `failed`/`timeout`). `connector:manage`, own tenant. Body (all optional; `deny_unknown_fields`): `{schema, glob, search, limit (1..10000, default 10000), object}`; `object` required for preview, refused for list; strings <= 256 chars, no control chars; 400 otherwise.
  - SQL list: no `schema` -> schemas (`kind: "schema"`); with `schema` -> base tables named `schema.table` (`kind: "table"`). Preview `object` = `schema.table`: columns with source types (`inferred:false`) and `primaryKey`.
  - Files (S3 with `endpoint`): list by `glob` against the FULL key (`*` not across `/`, `**` across, `?`), narrowed by the connector `prefix`; `kind: "file"`, `size`, `modifiedAt`; CSV (head, 1,000 rows, types inferred) and Parquet (footer, declared types, <= 64 MiB) preview; JSON preview `not_supported`; no `endpoint` -> `not_supported`.
  - REST: list = dial endpoint paths (`kind: "endpoint"`), answered inline; preview goes to the orchestrator.
  - MongoDB, Kafka, SFTP, Oracle (and REST preview): answer `{"kind","state":"running"[,"scope","object"]}`; poll with GET.
- Answer (all routes): `{kind, state: running|done|failed, scope?, object?, objects?: [{name, kind: schema|table|file|endpoint|topic|collection, size?, modifiedAt?}], truncated?, columns?: [{name, typeName, inferred}], primaryKey?, errorClass?, reason?, startedAt?, endedAt?}`. `reason` is a fixed sentence for the class, or "Only the first 10,000 are shown..." when `truncated`. Failures are HTTP 200 with `state: "failed"`.
- Closed `errorClass` list: `unreachable`, `sign_in_refused`, `address_refused`, `no_permission`, `timeout`, `too_many_objects`, `not_supported`, `failed`. D10 names six; `address_refused` and `failed` are added (a refused address is actionable; an unclassified failure is said as one). Needs the planner's confirmation.
- Orchestrator: `POST /api/connectors/{id}/discovery-results` (`ingest:read` + service identity/unrestricted admin; body limit 4 MiB): `{requestId, kind, state: done|failed, errorClass?, objects?, truncated?, columns?, primaryKey?}`; answer `{recorded: bool}`. Run config of `discover_job`: `{"ops":{"run_discover":{"config":{connector_id, request_id, kind, object, schema, glob, search, limit}}}}`.
- Bounds: 10,000 objects, 2,000 columns, 3 MiB JSON (orchestrator posts <= 2.5 MB), 100 samples, 1,000 CSV rows, S3 scan 100,000 keys (then `too_many_objects`), SFTP directory 100,000 entries. Listing call limit 120 s; the route timeout for the two discovery paths is 130 s.

**Deviations and decisions for the planner.**
1. Two classes added to D10's six (above).
2. A preview with nothing to sample (empty topic/collection/file) is `columns: []` (D6 "never invented"), not a failure.
3. The old `POST /discover?schema=` stays; it lost views (D8) and its wrong texts (D10). Oracle/other types get fixed sentences pointing at the new routes.
4. `futures = "0.3"` added to `lakehouse-api` (already in `Cargo.lock`; one line added to that crate's dependency list in the lockfile) to stop a bucket listing at its bound.
5. MongoDB `code 18` now counts as a sign-in refusal in `ingest_errors.is_authentication_refusal`, so a load with a wrong Mongo password is no longer retried (behaviour change for loads).
6. Adapters refactored to share connection code (`mongodb.connect`, `sftp.resolve_host/open_session`, `oracle.connection_parts`, `kafka.check_brokers`, `rest._prepare`); load behaviour unchanged, existing tests pass.
7. S3 wrong secret key surfaces as `no_permission` (object_store reports 403 for both); S3 keys over 256 characters are not listed.
8. Kafka preview uses a per-request group id `lakehouse-discover-<requestId>` with manual assignment and no commit.
9. Oracle listing tables other than the login's own are listed, but the loader (`oracle.py`) loads `name.split(".")[-1]` without schema; tasks 5-8 should know.
10. A second click launches a second job (no de-duplication); latest request wins.

**Most likely to fail in CI.** (a) `sqlx::test`/route tests never run locally: the seed `INSERT` in `connector_discovery_routes.rs` (`CASE WHEN $4::text ...`) and the `connector` CHECK constraints for `ingest_mode`/`type`; (b) `route_auth` loops over the new `{kind}` routes; (c) MySQL/SQL Server listing queries are untested live (g6 gates later); `ESCAPE '\'` on MySQL/MSSQL; (d) PostgreSQL `ILIKE ... ESCAPE` typing of `$1 = ''`; (e) `cargo deny`/`--locked` on the hand-edited `Cargo.lock` line; (f) the g6 discover steps for files/REST/Kafka check `supported:false` on the OLD route, whose words changed.

### Tasks 5 to 8 (developer)

Branch `feat/phase-1-src-13`, base `547d57ea`. Commits: `4dd607c0` (task 5, REST one endpoint per object, and Oracle schema honoured: review 1 SHOULD-FIX 1), `ce4ab052` (follow-up to it, upper-case Oracle names), `cb4dee50` (task 6, Kafka topic save), `2609cfdd` (task 7, console, and the old route removed), `9bfaf58a` (task 8, gates and the new job). No migration.

**Task 6 answer.** SEC-14's re-point rule does NOT count a topic change as a target change: `Dial::target_identity` compares a Kafka connector's `bootstrapServers` only (store test `a_kafka_dial_is_re_pointed_by_its_bootstrap_servers_only` asserts `topic` leaves the identity unchanged; the console copy `target-identity.test.ts` too). The save is the existing `PUT .../ingest-spec` with the stored spec sent back and only `dial.topic` changed (`src/lib/connectors/kafka-topic.ts`), no credential. A test pins it with the console's own rule.

**Commands run (outcomes).**
- `(cd dagster && python -m pytest dispar_orchestrate -q -m "not loader")`: 986 passed (985 after task 5, 986 after the follow-up; this work added 2 REST and 4 Oracle tests; the count at the merge base was not taken before I started, so the difference from the tasks 1 to 4 handoff's 968 is not all mine).
- `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings` (shared target, jobs 2, sources touched first, detached): finished clean in 3m16s, on the tree of `2609cfdd` (the only Rust files after it are none). `rustfmt --edition 2024 --check` on the four touched Rust files: clean.
- `bun test src/features/connectors src/lib/connectors src/services`: 396 pass, 0 fail (31 files). The whole console suite was NOT run (killed by memory on this machine).
- `bun --bun tsc --noEmit`: clean. `bun --bun eslint src/features/connectors src/lib/connectors src/services next.config.ts`: clean (one `react-hooks/incompatible-library` warning for `useVirtualizer`, silenced with a disable comment naming the same use in `data-table.tsx`). The whole-repo `bun run lint` was not run (memory).
- `bash scripts/ci/tests/test_required_gate.sh`: 48 passed, 0 failed (includes two new cases for the new job). `python3 ops/lint/check_connector_gate_coverage.py`, `check_intra_package_imports.py`, `check_bare_iceberg_count.py`, `check_compose_init_readiness.py`: OK. `pytest ops/gates ops/lint`: 52 passed. YAML of `ci.yml` and the g6 override loads; every `ops/g4`/`ops/g6` Python file compiles.

**Not run.** `cargo test` (forbidden): no Rust test, and the Rust change here is only the removal of a route (below). Every gate (`g4`, `g6`, `g6-oracle`, the new job): they need containers. `docker compose config` for the new service and job (docker is off limits here): only YAML-loaded. The console in a browser. The whole console test suite and whole-repo lint (memory).

**Wire changes.**
- Console contract: `DiscoverResult`, `DiscoveredObject`, `DiscoveredColumn` and `ConnectorService.discoverConnector` are removed; `DiscoveryKind`, `DiscoveryState`, `DiscoveryErrorClass`, `DiscoveryObjectKind`, `DiscoveryObject`, `DiscoveryColumn`, `DiscoveryAnswer`, `DiscoveryRequest` and `startDiscovery` / `getDiscovery` are added, mirroring the store's `Answer`.
- API: `POST /api/connectors/{id}/discover` and its `POLICY_TABLE` row are removed (route and policy together, so `route_auth` stays consistent). **The handler `routes::connectors::discover` and `connector_discover.rs` stay** because the assistant's `discover_source` tool (`routes/ai/tools/ingest.rs`) calls the handler in-process. Moving that tool to the new listing changes its answer shape and `tests/fixtures/tool_schemas.json`, which is the AI team's work; doc comments at both places say so and "do not mount it again". Planner decision needed: leave, or schedule the tool's move and then delete `connector_discover.rs` (872 lines).
- `next.config.ts`: `experimental.proxyTimeout: 130000`. Next's `/api` proxy gives up after 30 s (`proxy-request.js`: `proxyTimeout || 30000`), so without it a 120 s listing would fail in the console while the API was still answering.
- REST loader: a source object whose `name` is a declared endpoint's `path` loads only that endpoint; any other name (saved before) loads every endpoint. The console writes the path as the name.
- Oracle loader: `SCHEMA.TABLE` passes `schema=` to dlt's `sql_database` (two schemas in one call: `ValueError`); table names go through the dialect's `normalize_name` (see deviation 2).
- Gates: the g6 REST source object is now `/items`; the Oracle gate's is `G6_INGEST.ORDERS`.

**Tests added or changed.** New: `run-discovery.test.ts` (5), `kafka-topic.test.ts` (3), `table-finder.test.tsx` (18: schemas into a datalist, fixed failure words, local filter, windowed rows of 5,000, server search after the pause and its clear, truncated list, orchestrator wait and failure and Cancel, glob listing with size and date, REST endpoint by path, Select shown vs Kafka radio, inferred note, declared types and key, empty topic, failed preview and retry, previews strictly one at a time), 8 panel tests (Kafka topic save without credential, Oracle find and add by name, no add-by-name for SQL, columns read once on tick, REST old-object note, REST pick by path, Google Sheets keeps the typed field), 2 REST and 4 Oracle adapter tests, 2 contract tests. Changed: the panel stub serves `discovery/list` and `discovery/preview`; the first test asserts the new call; four column-choice tests select the section's `summary` (the new "Columns" buttons also match `/^Columns/`) and wait for the columns that now arrive after the click; **the SRC-11 test "says to find the table first when its columns have not been listed" is replaced by two** (a saved table's section reads its columns through the preview route; a failed read says so and keeps the left-out list), because SRC-13 lifts that limit by design. Preview of a table a test does not name in `discovered` answers failed, so tests that type a key for a saved table keep the typed field.

**Gate job.** `g6-discovery-scale` (`ops/g6/src13_discovery_scale_test.py`, runner `src13-scale-test-runner` in the g6 override, profile `src13-scale`): API + its PostgreSQL + ClickHouse + oidc-mock only. Estimated 8 to 12 minutes (images ~1 to 2, stack ~3 to 4, 10,000 `CREATE TABLE` in batches of 500 ~1 to 2, calls seconds); `timeout-minutes: 20`. Registered in `ci-required` needs and env, `required_gate.sh` and its self-test, `docs/CI.md`. Existing jobs: the `discover` steps add Dagster `discover_job` runs after the loads: about 7 in `g6-ingest-other` (SFTP 2, MongoDB 2, Kafka 2, REST preview 1) and 3 in `g6-oracle` (schemas, tables, preview), each a short run; I estimate +2 to 4 minutes for `g6-ingest-other`, +1 to 2 for `g6-oracle`. I have no timing of those jobs after `SRC-9` to confirm they stay under 20 minutes: if `g6-ingest-other` was near the line, move the SFTP/MongoDB/Kafka/REST discover steps to a job of their own.

**Deviations and decisions for the planner.**
1. The old route is unmounted, not deleted outright (assistant tool, above).
2. A second Oracle defect found while writing the gate: the listing returns Oracle's own upper-case spelling (`EMPLOYEES`), but `MetaData.reflect(only=...)` (dlt) compares against SQLAlchemy's lower-case report of upper-case names, so a listed name would raise "requested table(s) not available". Fixed in `ce4ab052` with the dialect's own `normalize_name` (a mixed-case name is passed as it is). This is a second commit for task 5's finding, not a change of the plan.
3. "Add by name" stays for object storage, SFTP, MongoDB, REST (a typed name is the only way past a cut list, and the existing REST key test types a name), and is new for Oracle; SQL and Kafka have none. REST's hint says to type a declared path.
4. Server search rule (D9 says "more than a page"): a list is searched on the server when the API cut it (`truncated`) or when it answers in the call (SQL, object storage, REST) and holds more than 500 names; the orchestrator's types filter the whole list locally unless it was cut, because each search there would be a job. Rows are windowed past 200.
5. The SQL columns are read ahead (preview route, one call each) when a table is ticked or its mode needs columns; the types the orchestrator lists read columns only when "Columns" is pressed, so ticking never starts a job.
6. Oracle column choice stays off: `validate_excluded_columns` refuses Oracle (store test), so the SRC-11 comment now says so instead of "Oracle follows".

**Most likely to fail in CI.**
(a) `g6-oracle`: the load of `G6_INGEST.ORDERS` (schema argument plus `normalize_name`; the first time the loader meets a schema-qualified Oracle name), and the three orchestrator discover runs; `schema_observer` / catalog registration with an upper-case dotted name is untested live. (b) `g6-ingest-other`: the orchestrator listings in the gate stack (Kafka preview group id and manual assignment, MongoDB `listCollections` for the reader user, SFTP name without the path, REST first page), the REST load with `/items`, and job duration. (c) `g6-discovery-scale`: new job never run; the compose `up` of `lakehouse-api` with only the g6 override and `--profile src13-scale` (the `${VAR:?}` set copied from `g6-ops`), `psycopg2-binary` in the Dagster image, `conn-pg-lakehouse` resolving `env:POSTGRES_PASSWORD` in this stack, the exactly-10,000 and not-truncated assertion, the 120 s limit itself. (d) `g4-cdc`: the PostgreSQL CDC connector through the new routes (schema list then `p5_cdc`). (e) Rust CI: only the unmounted route; if a `route_auth` or policy test still names `/discover` it will say so.

### Fix round after CI 84a2f212 (developer)

Branch fast-forwarded to `origin/feat/phase-1-rest` (84a2f212) first.

**BLOCKER 1, Kafka discover (cause).** `_real_kafka_consumer` (`dagster/dispar_orchestrate/discover.py`, the `KafkaConsumer(...)` call near line 467) passed `api_version_auto_timeout_ms=10_000`. kafka-python 3.0.11 has no such key in `KafkaConsumer.DEFAULT_CONFIG`; the constructor raises `KafkaConfigurationError: Unrecognized configs` (reproduced here with the installed library). The listing caught it as an unclassified exception and posted `failed`. The loader's consumer (`ingest_factory.py:962`) never passed the key, which is why the load worked. Fixed by removing the key. `run_request` now logs an unclassified failure at WARNING with module and type name only (it was INFO, and the Dagster run's INFO line carried only the class). Tests: the consumer is built through a fake that refuses keys outside the real `DEFAULT_CONFIG` (verified to fail with the old key), and the warning carries the type name and not the message. `pytest dispar_orchestrate -m "not loader"`: 988 passed.

**BLOCKER 2, route_auth 404 (cause).** The tenant gate, not a missing connector and not the body: `require_connector_in_tenants` is the route layer of every `/api/connectors/{id}/*` route and answers 404 for a connector outside the caller's tenants; `create_principal_with_permissions` makes a user with no tenant, so the gate answered 404 before the handler could answer 400 for the missing `object`. Fixed in the test: both principals join Meridian Group (`conn-pg-lakehouse`'s tenant), as `ingest_spec_put_rejects_a_dial_whose_host_resolves_internal` does. The policy assertions are unchanged (403 for `ingest:read` on the person's routes, 403 on the result route for both); with the tenant joined the 403 now provably comes from the permission gate. The route is unchanged.

### SEC-11 guard fix (developer)

`no_route_builds_a_response_from_an_upstream_errors_text` flagged three lines of `routes/connectors.rs`. Read from the `Display` implementations: (1) `secret_store_error`'s `Io | Key` arm (SRC-12 A1): `SecretStoreError::Io` prints a fixed sentence plus only the `std::io::ErrorKind` variant name (`{0:?}` of an enum, not the OS message); `Key` prints a `CryptoError`, every variant a fixed sentence naming the setting. No upstream text, so it is allowlisted, not rerouted through `upstream_error`. (2) and (3) `InvalidExcludedColumns` (SRC-11 D9 to D11), 400 for `BadList` and 422 for the rest: every variant is a fixed sentence in `ingest_spec.rs` that echoes only the object and column the request itself sent. Three `ALLOWED` entries with those reasons (the old `Io(_) => ...` entry matched no line any more and is replaced); each site carries a `// SEC-11 allowlisted` comment, which also makes the entry's substring unique to the line. The guard is otherwise unchanged. Clippy for `lakehouse-api` all targets is the check; the guard test itself was not run (`cargo test` forbidden).

## Review

### Reviews, 2026-10-15 (planner)

**Review 1, tasks 1 to 4.** Accepted: the error classes `address_refused`
and `failed` beyond the six of decision 10; an empty preview is `columns:
[]`; a MongoDB sign-in refusal (code 18) is not retried; the adapter
refactors that let listings share the loaders' connection code.
`SHOULD-FIX 1`: the Oracle loader kept only the table part of a listed
`SCHEMA.TABLE`, so a listed table from another schema would not load —
fixed in task 5, plus a second Oracle defect the developer found (upper-case
names against the dialect's lower-case reflection).

**Review 2, tasks 5 to 8.** Accepted: the old `/discover` route is unmounted
but its handler stays, because the assistant's `discover_source` tool calls
it in-process; moving that tool is assistant work outside dashboards, filed
as `SRC-22` for the AI team. Search runs on the server above 500 names
(decision 9's "more than a page"). `BLOCKER 1` (CI): a store test assumed an
empty `public` schema — fixed with a schema of its own.

**Review 3, after CI on `84a2f212`.** `BLOCKER 1`: Kafka listing passed a
setting kafka-python 3.0.11 does not know, so the consumer was never built;
fixed, and an unclassified listing failure now logs its type name.
`BLOCKER 2`: a route-permission test's principals had no tenant, so the
tenant gate answered first; fixed without weakening the policy check. Then
`main`'s `SEC-11` guard flagged three of this phase's lines as returning an
error's text; each is the repository's own fixed sentence (checked by the
reviewer for the credential store's I/O error, which carries only the error
kind) and is allowlisted with its reason.

CI on #144 at `f2632d21` (run 38692942055): every gate passes, including the
real discover step of all eleven types and the 10,000-table job (listing in
0.06 s, `docs/plans/SRC-13-RESULT.md`); the only red check was the `SEC-11`
guard, fixed in `9aff013d`, whose CI is running. No open `BLOCKER`. Open for
the product owner: the browser check of the Find flow at the end of the
phase.

### Browser check, 2026-10-16 (product owner)

Passed on the QA instance built from #144 at `ff7616e9`. The instance shares
an older orchestrator, so runs, loads, orchestrator-run listings,
secret-manager references and OAuth sign-in were not reachable there; those
are proven by the CI gates of #144.
