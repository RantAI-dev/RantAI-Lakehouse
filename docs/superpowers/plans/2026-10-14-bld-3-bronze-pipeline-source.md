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

Product and fix commits, newest first; the `BLD-3 handoff: …` commits
that carry this section are interleaved with the commands they record and
are omitted here (they touch only this file).

```
2c5b19d4 BLD-3 T5 fix: g7 prose tripped the R11 bare-count guard
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

1. **Rust block, on `f85b7a72` (the branch head when it ran).**

   ```
   cd rust && cargo fmt --check                    # rc 0, no output
   cargo clippy --workspace --all-targets --all-features -- -D warnings
                                                    # rc 0, "Finished dev profile" after 37.93s
   cargo test --workspace                           # rc 0
   ```

   `cargo test --workspace`: **97 suites, 4742 passed, 0 failed, 0
   ignored** (full log in the session's `.local/logs/bld3-cargo-test.log`;
   no suite reported a non-zero failure count). `RUST_TEST_THREADS=4`,
   shared `CARGO_TARGET_DIR`, under `flock .heavy.lock`, no `cargo
   clean`.

   Before this, on the merge commit and the test commit separately:
   `cargo check -p lakehouse-api --all-targets` rc 0, and
   `cargo clippy -p lakehouse-api --all-targets -- -D warnings` rc 0.
   Targeted run of this slice's own tests,
   `cargo test -p lakehouse-api --lib -- bronze_target_is_refused
   validate_target_zone`: **4 passed, 0 failed, 0 ignored** (1643
   filtered out) — the `create`, `update` and `restore_version`
   refusals plus the pure rule.

2. **TypeScript block, same commit.**

   ```
   bun run typecheck        # rc 0, no output
   bun run lint             # rc 0 — 6 warnings, 0 errors (pre-existing,
                            # none in a file this slice touched)
   bun test, per folder     # pass=1284 fail=0 skip=2, no folder exited
                            # non-zero (printed as `nonzero:` with nothing
                            # after it)
   ```

   The frontend tests were run per folder (`bun test <dir>/` for each
   directory holding a `*.test.ts*`), under `.heavy.lock`, because a
   single `bun run test` process is what reaches 7.2 GB and gets
   killed on this machine. `skip=2` is the same two skips the full run
   reports; no test folder failed.

3. **Python lint scripts — FAILED on this branch's own code, fixed,
   re-run green.**

   ```
   python3 ops/lint/check_intra_package_imports.py   # rc 0
   python3 ops/lint/check_bare_iceberg_count.py      # rc 1 (see below)
   ```

   First run of `check_bare_iceberg_count.py` exited 1 with two
   findings, both on lines **T5/D7 added** to
   `ops/g7/g7_pipeline_test.py`:

   ```
   ops/g7/g7_pipeline_test.py:362: bare count() against an Iceberg/Bronze table (R11)
   ops/g7/g7_pipeline_test.py:401: bare count() against an Iceberg/Bronze table (R11)
   ```

   Neither is SQL. Line 362 is the new step's own docstring prose
   (`must hold exactly \`count(DISTINCT id)\` rows`), line 401 is a
   `G7Failure` message; the guard is a +-800-char text heuristic, so
   both pair a literal `count(` with `bronze.`. The guard's own
   docstring names the fix for this case — *"reword the comment, not
   weaken the check"* — so both were reworded in their own commit
   (rule 2: the check was not touched). The SQL never changed and was
   already right: the Bronze side carries `WHERE 1` and reads through
   the gate's `DataLakeCatalog` database, the Silver side is a plain
   MergeTree table.

   ```
   python3 ops/lint/check_intra_package_imports.py   # rc 0
   python3 ops/lint/check_bare_iceberg_count.py      # rc 0
   python3 ops/lint/check_bare_iceberg_count.py      # "R11 guard: no bare
                                                    #  count() against a Bronze
                                                    #  Iceberg table found."
   ```

4. **Dagster tests.**

   The venv the earlier session was said to have made does not exist
   (`/home/shiro/rantai/venv-bld3` and `dagster/.venv` are both absent,
   and system `python3` has no `pytest`), so it was rebuilt the way
   `.github/workflows/ci.yml` installs it, under `.heavy.lock`:
   `uv venv --python 3.11 /home/shiro/rantai/venv-bld3` then
   `uv pip install ./dagster pytest==8.3.4`.

   ```
   cd dagster && python -m pytest dispar_orchestrate -q
   # 1039 passed, 0 failed, 585 warnings in 931.46s (0:15:31)
   ```

   One suite, no skips reported, no failures. (The suite the plan's
   T2 check names, `test_authored_factory.py`, is inside it.)

5. **Compose config.**

   ```
   docker compose --profile '*' config --quiet   # rc 0, no output
   ```

6. **The real local run of the `g3a-dagster` job, from a clean
   project — PASS, twice.**

   `.local/ci-g3a-g7.sh` runs the job's steps in the job's order, on
   project `bld3`, profile `dagster`, under one `flock`: `down -v`;
   write the CI-only `.env` (the same six values `ci.yml` writes, deleted
   again at the end); build the two images at THIS commit plus the
   `oidc-mock` fixture; `up -d --no-build lakehouse-api
   dagster-code-location dagster-webserver dagster-daemon
   g3a-source-init`; `run --rm g3a-test-runner`; `run --rm
   g7-test-runner`; dump the code-location log; `down -v`.

   Run 1, from a clean project with the images built from scratch:

   ```
   GIT_SHA=a01e34dba77e6a66b4b3887be1b32d2f3e2264ab
   build rc=0 in 431s
   up rc=0 in 23s
   g3a rc=0 in 44s
   g7  rc=0 in 35s
   ```

   Run 2, same script, images rebuilt at the same commit from cache (5s),
   this time with the runners' output captured rather than swallowed by
   `>/dev/null` (run 1's log kept only the exit codes — a script bug,
   fixed, not a product change):

   ```
   build rc=0 in 5s
   up rc=0 in 22s
   g3a rc=0 in 40s
   g7  rc=0 in 35s
   SUMMARY g3a_rc=0 g7_rc=0 up_rc=0 build_rc=0
   ```

   g7's output lines, verbatim from run 2:

   ```
   [g7] logged in as bootstrap admin
   [g7] authored pipeline 'pl-g7-dedupe-select-mv86vkg9' created and activated
   [g7] ready: run 98db5d9f-9e98-49ab-9b3c-b74cd81ebe53 terminal
   [g7] run 98db5d9f-9e98-49ab-9b3c-b74cd81ebe53 reports real row materializations
   [g7] silver.g7_dedupe_select_out holds 2000 rows = the 2000 distinct ids of bronze.g3a_orders
   [g7] 'bronze_maintenance_job' graph has 3 op(s); source contains 'list_bronze_tables'
   [g7] PASS
   ```

   **g7 wall time: 35 s** (34 s in run 1, 30 s in the first session's run
   at `f8d4e76f`). That is the number `timeout-minutes: 30` in the CI job
   was set from, and it still holds on this branch's head after the
   merge of #80. g3a's own line for the record: `[g3a] PASS`, 40 s.

   Both runs ended in `down -v`; `docker ps -a --filter name=bld3` is
   empty afterwards, and `git status` shows only the untracked
   `.local/`.

   Extra diagnostic run for section 6 question 3 below
   (`.local/probe-dedupe.sh`, same clean-project shape, CI-only `.env`,
   ends in `down -v`): `.local/logs/probe-dedupe3.log`.

   The stack ran at `a01e34db`, which already contains every product and
   fix commit above (including the R11 reword, `2c5b19d4`). The only
   commits after it are commits to this plan file.

### Section 6 answers

**T1's real error text, verbatim** (from commit `4c5dabfb`, reproduced
on `main`'s code with a clean `-p bld3` stack, G3a first then g7):

```
[g7] authored pipeline 'pl-g7-dedupe-select-mv7tszyb' created and activated
requests.exceptions.HTTPError: 404 Client Error: Not Found for url:
  http://clickhouse:8123/
  File "authored_factory.py", line 443, in _run
    written = _write_clickhouse_table(ch, target_zone, target_table, select_sql)
  File "authored_factory.py", line 293, in _ensure_target_table
  File "bronze_catalog.py", line 210, in _ch_exec -> resp.raise_for_status()
Exceeded max_retries of 2  (run failed after 1m59s)
```

**1. Does `CREATE TABLE ... ENGINE = MergeTree ... AS SELECT ... FROM
<catalog db>.`bronze.t` LIMIT 0` infer the column types correctly?**
Yes. Measured on a running stack (`.local/probe-types.sh`,
`.local/logs/probe4.log`), on the exact shape the op sends with
`SETTINGS` last:

```
CREATE TABLE IF NOT EXISTS silver.`probe_all_cols` ENGINE = MergeTree
  ORDER BY tuple() AS SELECT * FROM icecat_authored.`bronze.g3a_orders`
  LIMIT 0 SETTINGS allow_database_iceberg = 1        -- status 200
DESCRIBE TABLE silver.`probe_all_cols`
  id             Int64
  customer       String
  amount         Decimal(12, 2)
  created_at     DateTime64(6, 'UTC')
  _ingested_at   Nullable(DateTime64(6, 'UTC'))
  _dlt_load_id   String
  _dlt_id        String                              -- status 200
INSERT INTO silver.`probe_all_cols` SELECT * FROM
  icecat_authored.`bronze.g3a_orders`
  SETTINGS allow_database_iceberg = 1                -- status 200
SELECT count() FROM silver.`probe_all_cols`          -- 2000
```

The `DESCRIBE` of the Iceberg source and of the MergeTree table it
created are identical, column for column: `Nullable` stays `Nullable`,
`Decimal(12,2)` stays `Decimal(12,2)`, timestamps stay `DateTime64(6,
'UTC')`. Also measured (D2's "put the setting where ClickHouse accepts
it"): `SETTINGS` **after** `LIMIT 0` is accepted; putting it before
`LIMIT 0` is a syntax error —

```
Code: 62. DB::Exception: Syntax error: failed at position 204 (LIMIT):
LIMIT 0. Expected one of: UNION, EXCEPT, INTERSECT, ... (SYNTAX_ERROR)
```

— which is why `_reading` appends `SETTINGS` to the whole statement
(`authored_factory.py:322`).

**2. Which ClickHouse user, and can it read the catalog database with
authz on?** `default`, and yes. `SELECT currentUser()` sent from inside
`dagster-code-location` returns `default` (`.local/logs/probe2.log`);
that is `bronze_catalog.py:78-79` reading `CH_USER`/`CH_PASSWORD`,
whose compose defaults are `default` and empty
(`docker-compose.yml:160-161`). With authz on (the compose default
installs `clickhouse-iceberg-init`, `clickhouse-oauth-init`), the same
identity created `icecat_authored` as a `DataLakeCatalog` and read
2000 rows out of it through `lakekeeper` + `oidc-mock` OAuth
(`catalog_credential`/`oauth_server_uri`), status 200. So the refusal in
the API and the op is the only place that needed to change; the read
identity needed nothing.

**3. Does `ORDER BY ... LIMIT 1 BY` dedupe behave the same over an
Iceberg source?** Yes — measured this session, because the first
session's measurement did not answer it. `.local/probe-dedupe.sh` sends
the shape `_build_select_sql` actually builds (`authored_factory.py:377`:
the dedupe on the OUTER select that reads the Iceberg table, `SETTINGS`
last) over a source doubled with `UNION ALL` so duplicates exist — the
G3a fixture's own ids are unique, so g7's green 2000 rows do **not**
exercise dedupe:

```
SELECT count() FROM (SELECT id, customer FROM icecat_authored.`bronze.g3a_orders`
                     UNION ALL SELECT id, customer FROM icecat_authored.`bronze.g3a_orders`)
                                                        -- 4000  (control, no dedupe)
SELECT count() FROM (SELECT id, customer FROM (
      SELECT id, customer FROM icecat_authored.`bronze.g3a_orders`
      UNION ALL SELECT id, customer FROM icecat_authored.`bronze.g3a_orders`)
      ORDER BY id LIMIT 1 BY id SETTINGS allow_database_iceberg = 1)
                                                        -- 2000  (the op's shape)
```

4000 → 2000: the dedupe collapses exactly as it does over a MergeTree
source. The earlier probe (`.local/logs/probe2.log`) measured **4000 and
4000** and so answered nothing — it had put `ORDER BY .. LIMIT 1 BY`
*inside* the subquery it then counted, a shape the op never sends. Named
here so the reviewer does not read that log as a contradiction.

**4. Any other caller that builds `bronze.<table>` as a ClickHouse name
for an authored pipeline?** No. Swept `src/`, `rust/crates/`,
`dagster/` and `ops/` for the broken shape: it existed only in
`authored_factory._resolve_source`, which T2 fixed. Every other Bronze
read already goes through a per-component `DataLakeCatalog` database:
`icecat_api` (`routes/catalog_source.rs:142`, the one canonical helper,
over `config.iceberg_query_db`), `icecat_ingest`
(`connector_catalog.py:59`), `icecat_maintenance` (`maintenance.py`),
`icecat_authored` (T2), and `icecat_g3a` / the gates' own `CATALOG_DB`
(`ops/g3a`, `ops/g7`, `ops/g6`, `ops/g8`, `ops/g9`). Reported, not
fixed, because none of them is broken:

- `src/lib/uploads.ts:292` builds `` `bronze.${table}` `` — that is the
  Iceberg namespace the API and Data Explorer use, not a ClickHouse
  database.
- `src/lib/asset-query.ts:39` can build `<namespace>.<name>` with
  `engine: "clickhouse"`, but a Bronze asset takes the earlier branch
  at `:37` (Trino) or the API's own `queryTarget`
  (`icecat_api.`bronze.<t>``); `:89` even refuses the snapshot link
  unless the name contains `` `bronze. `` inside a catalog database.
- `lake.`bronze_meta.*`` (`routes/catalog.rs`, `bronze_catalog.py`,
  `gold_export.py`) is the console's MergeTree **metadata registry**, a
  different database.
- `bronze.<table>` as a policy key / lineage node id
  (`routes/catalog.rs`, `routes/lineage.rs`, `sql_rewrite.rs`) is a
  key, never SQL.

And, to answer the shape of the question directly: **neither the console
nor the assistant builds pipeline SQL at all**, so neither can carry
this defect. The console collects and displays `sourceZone`/
`sourceTable` only (`pipeline-definition.tsx:281` says the orchestrator
builds the SQL from the transforms); `routes/mod.rs` registers no
preview/dry-run route for authored pipelines; the assistant's
`create_pipeline` (`routes/ai/tools/pipelines.rs`) forwards the body to
`routes::pipelines::create`, which persists it. What they *do* preview is
a user's own SQL, or a catalog asset through the API's catalog database.

### Not verified

- **`bun run test` as one process was deliberately not run.** On this
  machine that single process reaches 7.2 GB and is killed (the machine
  rule this session was given). The same files were run per folder:
  `pass=1284 fail=0 skip=2`, no folder exited non-zero. A CI job that
  runs the whole suite in one process is therefore *not* covered by
  anything measured here.
- **CI itself was not observed.** No PR was opened and no CI run was
  read. The local run in item 6 is the job's steps replayed by hand, not
  a green check on GitHub.
- **Compose: only the `dagster` profile was really brought up.** That is
  the profile this slice's new service (`g7-test-runner`, T1) belongs
  to, and it was really run twice, on clean projects. The other
  profiles were only `config --quiet`-checked. No profile outside
  `dagster` was changed by this slice.
- **`cargo test --workspace` includes doc-tests** (14 of the 97 suites);
  they are counted in the 4742. It is not a separate number because
  the workspace run reports them inline.
- **The pytest marker split CI uses was not reproduced as two runs.**
  Both halves were covered in one: `pytest dispar_orchestrate -q` with
  no marker filter ran all 1039, of which 258 carry `-m loader`
  (`--collect-only -m loader`: "258/1039 tests collected (781
  deselected)"). No test was deselected in the real run and nothing was
  skipped.
- **Masking across zones (D5) is untouched and unmeasured** — out of
  scope by decision, recorded by the planner as its own backlog item.
- **A stored pipeline with a `bronze` target left in the database** (the
  D4 case that is not migrated away) is asserted at the route level and
  in the op's unit tests, but no such row was seeded on a real stack
  and run end to end. What the stack run does cover is the
  `bronze`->`silver` happy path.
- **The first session's probe logs were read, not re-run.** Questions 1
  and 2 of section 6 are answered from `.local/logs/probe4.log` and
  `.local/logs/probe2.log`, which were measured at `ad3db91e`/`bcd9b546`,
  not at this head. Question 3 was re-measured at this head because the
  earlier measurement did not answer it (see section 6, Q3). Nothing in
  the merge of #80 or in the g7 reword touched any statement those two
  probes sent, and the g7/g3a run at this head exercises the same reads.

## 8. Review

(Reviewer.)
