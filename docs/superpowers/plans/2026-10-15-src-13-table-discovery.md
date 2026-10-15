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

## Review

*(planner)*
