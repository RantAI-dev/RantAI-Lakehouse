# `BLD-3` + `SRC-20`: a pipeline reads a Bronze table, and the pipeline gate runs in CI

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `8f93fc19`. Branch: `fix/bld-3-bronze-pipeline-source`.
One PR slice.

## 1. The problem

A pipeline built in the console (or by the assistant's `create_pipeline`)
from Bronze to Silver does not run. Verified by reading `main` at `8f93fc19`,
not yet reproduced on a stack (task T1 reproduces it):

- `dagster/dispar_orchestrate/authored_factory.py::_op_for_pipeline` builds
  its source as `f"{source_zone}.`{source_table}`"`, so a Bronze source is
  read as the ClickHouse table `` bronze.`orders` ``.
- ClickHouse has no `bronze` database (`scripts/seed-clickhouse.sql` creates
  `serving`, `silver`, `lake`). Bronze is Iceberg in Lakekeeper, readable
  from ClickHouse only through a `DataLakeCatalog` database, where a table is
  named `` <db>.`bronze.<table>` `` (`docker-compose.yml`,
  `clickhouse-iceberg-init`).
- The assistant's `create_pipeline` schema (`routes/ai/registry.rs`,
  `create_pipeline_schema`) offers `bronze` as a source AND as a target. A
  Bronze target makes `_ensure_target_table` run `CREATE DATABASE IF NOT
  EXISTS bronze` in ClickHouse: a MergeTree table that is not Bronze and that
  nothing else reads.
- The only end-to-end test of this path, `ops/g7/g7_pipeline_test.py`, is run
  by no CI job and has no compose runner (`SRC-20`). Its module doc says it was
  written without ever being run.

## 2. Decisions (planner)

- **D1. Read Bronze through the job's own `DataLakeCatalog` database.** When
  `sourceZone == "bronze"`, the op calls the existing helper
  `connector_catalog.ensure_catalog_database(ch, db)` with its own database
  name `icecat_authored` (same rule as `icecat_maintenance`, `icecat_ingest`:
  each job owns its own catalog database), and reads
  `` icecat_authored.`bronze.<table>` ``. Do not write a second copy of the
  `CREATE DATABASE ... DataLakeCatalog` statement (rule 4). If importing from
  `connector_catalog` breaks `ops/lint/check_intra_package_imports.py`, stop
  and report rather than duplicating it.
- **D2. Every statement that reads the catalog database carries
  `SETTINGS allow_database_iceberg = 1`**, as `maintenance.py` does. This
  covers the `CREATE TABLE ... AS SELECT ... LIMIT 0`, the staging `INSERT
  ... SELECT`, and any count. Put the setting where ClickHouse accepts it for
  each statement shape. Verify on the running stack, not from documentation.
- **D3. `silver` and `gold` sources stay as they are** (`<zone>.<table>` in
  ClickHouse).
- **D4. A Bronze target is refused** (fail closed, principle 3). Bronze is
  written only by ingestion. Refused in three places with the same message,
  `"a pipeline cannot write to bronze; bronze is written by ingestion"`:
  1. the API, when a pipeline is created, updated or restored (find the
     validation `update` and `restore_version` share, `validate_pipeline_input`
     or its successor, by symbol);
  2. the assistant's `create_pipeline` schema: `targetZone` enum becomes
     `["silver", "gold"]`. Regenerate `tests/fixtures/tool_schemas.json` the
     way that fixture's test says to;
  3. the job (`_op_for_pipeline`): non-retryable `Failure`, before any SQL.
  An existing stored pipeline with a Bronze target is left in the database;
  it now fails its run with that message. Do not write a migration for it.
- **D5. Masking is out of scope and stays as today.** A pipeline runs as the
  orchestrator's ClickHouse user and copies unmasked rows, the same as a
  Silver-to-Gold pipeline does today. Masking still applies when people query
  the output table. A policy on a Bronze column does not follow the row into
  Silver; that is an existing gap, recorded by the planner as its own backlog
  item, not fixed here.
- **D6. `SRC-20`: g7 runs inside the existing `g3a-dagster` CI job**, after
  the G3a test, because g7 needs the Bronze table `g3a_orders` that G3a loads.
  No new job (keep CI time down); the job's `timeout-minutes` may rise by the
  measured g7 time plus 5 minutes. Add a compose service `g7-test-runner`
  shaped like `g3a-test-runner` (profile `dagster`, `python:3.12-slim`,
  `PYTHONDONTWRITEBYTECODE: "1"`, read-only mount of `./ops/g7`, no default
  credentials).
- **D7. g7 proves the rows, not only a count field.** Add one assertion to
  g7: after the run succeeds, the Silver table `silver.g7_dedupe_select_out`
  has exactly `SELECT count(DISTINCT id)` rows of the Bronze table, both read
  through ClickHouse. A FAILURE run must fail the gate (today
  `steps_are_terminal` accepts FAILURE as terminal; make sure a failed run
  raises `G7Failure` with the run's failure message). Remove the stale module
  doc paragraphs saying the routes do not exist, after checking that they
  exist on this branch.

## 3. Anchors (`main` at `8f93fc19`)

- `dagster/dispar_orchestrate/authored_factory.py`: `_op_for_pipeline`
  (source built near line 441), `_build_select_sql`, `_ensure_target_table`,
  `_write_clickhouse_table`, `CONNECTOR_SOURCE_UNSUPPORTED_REASON` (the shape
  for a new reason constant).
- `dagster/dispar_orchestrate/connector_catalog.py`: `ensure_catalog_database`,
  `CATALOG_DB` naming precedent.
- `dagster/dispar_orchestrate/maintenance.py`: `_ensure_catalog_database`,
  `discover_bronze_tables` (how `bronze.<table>` names look in the catalog db).
- `dagster/dispar_orchestrate/test_authored_factory.py`: `OpForPipelineTest`.
- `rust/crates/lakehouse-api/src/routes/ai/registry.rs`: `create_pipeline_schema`.
- `rust/crates/lakehouse-api/src/routes/pipelines.rs` and
  `routes/authored_pipelines.rs`: create, `update`, `restore_version`.
- `docker-compose.yml`: `g3a-test-runner` (shape), `dagster-code-location`
  env (it already has `LAKEKEEPER_CATALOG_URI`, `LAKEKEEPER_WAREHOUSE`,
  `CH_RUSTFS_S3_ENDPOINT`, `CH_OAUTH_CLIENT_ID`, `CH_OAUTH_SERVER_URI`).
- `.github/workflows/ci.yml`: job `g3a-dagster`.
- `ops/g7/g7_pipeline_test.py`.

## 4. Tasks (one commit each, in order)

- **T1. Reproduce first.** Bring up the G3a stack locally exactly as the
  `g3a-dagster` CI job does (`-p bld3`, profile `dagster`), run the G3a test,
  then create and run a Bronze-to-Silver pipeline (g7 does this). Record the
  real error text in the handoff. No commit if nothing changes; if you add the
  compose runner here, commit it as T1.
  *Check:* the error you saw is quoted in the handoff. If the pipeline
  *succeeds* on `main`, stop and report: the plan is wrong.
- **T2. D1 + D2 in the job,** with unit tests in `test_authored_factory.py`
  (no network, `_ch_exec` mocked as the existing tests do):
  a Bronze source reads `` icecat_authored.`bronze.<t>` ``; the catalog
  database is ensured before the first read; every statement reading it
  carries `allow_database_iceberg = 1`; a Silver source is unchanged.
  *Check:* `(cd dagster && python -m pytest dispar_orchestrate -q)` and the
  two `ops/lint` scripts pass.
- **T3. D4, the job half,** plus its unit test (Bronze target → non-retryable
  `Failure`, no SQL sent).
- **T4. D4, the API and assistant half.** Rust: refuse a Bronze target on
  create, update and restore, with a test for each; schema enum; regenerated
  fixture. *Check:* `cargo fmt --check`, `cargo clippy -p lakehouse-api
  --all-targets -- -D warnings`, `cargo test -p lakehouse-api`.
  If the console's pipeline form offers Bronze as a target, remove it there
  too (`src/`), with its test; then `bun run typecheck && bun run lint &&
  bun run test`.
- **T5. D6 + D7: g7 in CI.** Compose runner, the g7 changes, the CI step.
  *Check:* the real local run of the whole `g3a-dagster` sequence passes,
  G3a then g7, from a clean project (`down -v` first), and you quote g7's
  output lines and its wall time.
- **T6. Docs.** `CHANGELOG.md` entry a customer can read; in
  `docs/core/BACKLOG.md` do NOT change status (the reviewer does after merge).

## 5. Out of scope

Masking across zones (D5), uploaded-table sources (`DATA-19`), connector
sources (`CONNECTOR_SOURCE_UNSUPPORTED_REASON` stays), joins (`BLD-1`),
incremental loads.

## 6. Verify, do not assume

1. Does `CREATE TABLE ... ENGINE = MergeTree ... AS SELECT ... FROM
   <DataLakeCatalog db>.\`bronze.t\` LIMIT 0` infer column types correctly
   (Nullable, timestamps)? Quote a `DESCRIBE` of the Silver output.
2. Which ClickHouse user does the code location use, and can it read the
   catalog database with authz on (the compose default)?
3. Does the `ORDER BY ... LIMIT 1 BY` dedupe behave the same over an Iceberg
   source? (g7 uses `dedupe(id)`.)
4. Is there any other caller that builds `bronze.<table>` as a ClickHouse
   name for authored pipelines (the console's preview, the AI tools)? List
   them; fix only the job, report the rest.

## 7. Handoff

Developer: a non-Opus agent (`AGENTS.md`). Two sessions: the first
built and pushed T1–T6 and reproduced T1, then stopped before the
verification block; this session merged `origin/main` (PR #80) per the
planner's instruction and ran the block.

### Commits (`origin/main..HEAD`)

```
f85b7a72 BLD-3: tests follow PR #80's route signatures
e055c335 merge: main into fix/bld-3-bronze-pipeline-source (bring #80 in)
61fb0a58 BLD-3 T6: changelog entry for reading a Bronze table and refusing a Bronze target
9596092d SRC-20 T5: run g7 in CI, and make it prove the rows and fail on a failed run
f8d4e76f BLD-3 T4: refuse a bronze target in the API and in the assistant's schema
bcd9b546 BLD-3 T3: refuse a bronze target in the job, before any SQL
ad3db91e BLD-3 T2: read a Bronze source through the job's own DataLakeCatalog database
4c5dabfb BLD-3 T1: reproduce the Bronze-source failure; add the g7-test-runner service
5442bf34 BLD-3 docs(plan): read Bronze through the catalog; refuse Bronze targets; g7 in CI (SRC-20)
```

### The merge of PR #80 (5408c797) — order, decided

One conflict file, `rust/crates/lakehouse-api/src/routes/authored_pipelines.rs`,
six hunks, resolved hunk by hunk (AGENTS rule 9), both sides kept in
every hunk:

- D4's `BRONZE_TARGET_REFUSED` / `validate_target_zone` stand next to
  #80's F2.2 tenant helpers.
- `update` keeps `validate_target_zone`; this branch's inline
  transform-grammar and `max_retries` loop is dropped because #80's F2.8
  moved both into the shared `validate_pipeline_input` that `update` and
  `restore_version` now call. Nothing is lost: the refusal is the only
  check that is still route-local.
- `restore_version` likewise.

**Order: `in_scope` (F2.1, #80) first, `validate_target_zone` second**,
on both routes. So an out-of-scope id answers the same 404 as an
unknown id whatever the body or the snapshot holds — the existence
answer never depends on the caller's payload — and the Bronze refusal is
the next answer for an in-scope id. The refusal cannot be moved ahead of
`in_scope` without handing an out-of-scope caller a body-dependent
answer, which is the one thing #80's 404 exists to prevent.

This branch's two route tests for the refusal were re-shaped in the
next commit (`f85b7a72`): `update`/`restore_version` need a pool to get
past `in_scope` at all, so both now seed a row and call as
`unrestricted_principal()`.

### Commands, in the order the block runs them

(Each entry is appended as the command finishes.)

### Section 6 answers

(Appended after the run.)

### Not verified

(Appended at the end.)

## 8. Review

(Reviewer.)
