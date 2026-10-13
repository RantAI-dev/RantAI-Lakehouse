# `SRC-8` Schema changes at the source — Implementation Plan

> **Migration numbers, 2026-10-10.** `main` took `0061` after this plan was
> written. This branch's migrations were renumbered in `36f2a2a`, before any
> was applied: `0061`→`0062` (`SRC-6`), `0062`→`0063` (`SRC-7`),
> `0063`→`0064` and `0064`→`0065` (`SRC-8`). Numbers below are the old ones
> where they record what was done at the time.

**Status:** ready to build. Decisions D1–D9 on the feature page were signed
by the product owner on 2026-10-09, all as proposed.
Written 2026-10-09 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/src-8.md`. Backlog `SRC-8`.
**Feature page:** `docs/core/features/source-schema-changes.md`. Decision
numbers below (D1–D9) are its rows.

**Base and branch:** `feat/phase-1-connector-reliability`, on top of `SRC-7`
(`9088b69`). One branch for Phase 1, not pushed until the phase is done. No
Rust test of `SRC-6` or `SRC-7` has run yet.

**Why.** A source table that changes shape changes the Bronze table silently.

---

## 1. What exists today (anchors verified at `9088b69`)

| # | Spec row | Code | Verdict |
| --- | --- | --- | --- |
| F1 | Detection: default evolution, silent | `dagster/dispar_orchestrate/adapters/sink.py` (`load_via_sink`) calls `dlt.pipeline(...).run(...)` with no schema contract and compares nothing. `ingest_factory.py` (`_run_one_object`) records rows and status per table, never columns. | Confirmed. |
| F2 | Per-connector policy: none | `connector` has no such column; `UpdateConnectorInput` (`lakehouse-store/src/connectors.rs`) has no such field. | Confirmed. |
| F3 | Breaking changes: not detected | Nothing reads a source table's columns before loading. `sql_database(...)` reflects them (`adapters/sql.py`, `dlt_pipeline.py`, `adapters/oracle.py`) and the result is used only to load. | Confirmed. |
| F4 | Removed columns: not handled | dlt never drops a column, so the column stays; nothing marks it. | Half of the target holds by accident; the mark does not exist. |
| F5 | Notice: none | The connector page has no such list. `SRC-7` built the rule kind `connector_schema_change` and `lakehouse_alerts::evaluate_connector_event`; nothing calls it for that kind. | The alert half is ready and waits for this work. |
| F6 | *Not in the spec* | A table cannot be held back: a run loads every selected table. A connector cannot be stopped: the schedule sensor asks `GET /api/connectors/ingestible` for due connectors and `POST …/ingest/run` starts any. | The pause the spec asks for needs both. `SRC-11`'s auto-disable needs the connector half too. |
| F7 | *Not in the spec* | What dlt does with a changed type was never measured here. | Task 2 measures it and writes `docs/plans/SRC-8-RESULT.md` before D9's wording ships (principle 5). |
| F8 | *Not in the spec* | No Python environment with dlt and Dagster exists on the build machine; `SRC-7`'s sensor tests ran against a stub. | This work is mostly Python. Task 1 builds the environment CI uses, once, outside the repo. |

## 2. Decisions already made by the planner

- **The API decides, the orchestrator observes.** Before a table loads, the
  orchestrator posts the columns it sees to the API. The API owns the last
  accepted columns, the comparison, the policy, the pause, the notice and the
  alert, and answers what to do with the table. The comparison and the policy
  are pure Rust functions with their own unit tests.
- **One answer per table**: `load` (optionally with the list of columns to
  load), or `wait`. The orchestrator obeys it and records the table as
  `waiting` in `ingest_run`; a `waiting` table does not fail the run (D7).
- **Fail closed.** If the API cannot be asked, the table is not loaded and
  the step fails. A type change on neither list is breaking (D3).
- **A connector-level pause** (`connector.paused_reason`) honoured by the
  schedule query and by `POST …/ingest/run`. Policy "pause" sets it;
  `SRC-11` will reuse it.
- **Holding a column back** uses the SQL source's own column selection, so
  `SRC-11`'s column choice can reuse it.
- **Inactive is a fact in the console's database**, shown where columns are
  listed. The Bronze table itself is not altered.

## 2a. Spec row to tasks

| Spec Target row | Tasks |
| --- | --- |
| Detection, on every run | 3, 4, 6, 7 |
| Per-connector policy, four choices | 3, 5, 8, 10 |
| Breaking changes always pause the table and ask | 4, 5, 6, 9 |
| Removed columns kept and marked inactive | 5, 11 |
| Notice on the connector page, and an alert | 5 (alert), 10 (notice) |

Spec acceptance lines: "add a column" → 4, 6, 10; "drop a column" → 4, 5, 6,
10; "set pause" → 5, 8; "refused without permission, honest failure" → 5, 9
and the fail-closed rule.

## 3. Tasks

One task per commit, in order. Rust tasks 3–5 and 9 sit together; the
migration lands with task 3.

1. **Python environment (no commit).** Create a virtual environment under
   `~/.cache/` the way CI's "Dagster · unit tests" job installs the code
   location and its test dependencies; run the existing suite once and record
   the count. Check `df -h /` first. If it cannot be built, stop and report:
   this work is not to be written against a stub.
2. **Measure what the loader does with a changed type (F7).** A unit test
   that runs the sink's pipeline against a local filesystem destination, in
   the style the existing sink tests use, for: a new column; a column that
   stops arriving; integer to text; text to integer; integer to a larger
   integer. It asserts what the table's columns are afterwards. The planner
   writes the result into `docs/plans/SRC-8-RESULT.md`. If a result
   contradicts D9, stop and report before going on.
3. **Store.** Migration, next free number (0063 at `9088b69`), why-header:
   `connector.schema_change_policy` (text, default
   `apply_non_breaking`, check on the four values) and
   `connector.paused_reason` (nullable text) with `paused_at`;
   `connector_source_schema` (the last accepted columns per connector and
   table); `connector_schema_change` (one row per change: connector, table,
   kind, column, before, after, breaking, status `applied` / `pending` /
   `approved`, run id, times, who decided). Store functions for each, bound
   parameters only. `Connector` and its TypeScript contract gain
   `schemaChangePolicy`, `pausedReason`, `pausedAt`.
   *Check:* store tests for every function, including "a second identical
   observation writes nothing".
4. **The comparison and the policy, pure.** In `lakehouse-store` or a module
   of `lakehouse-api` with no I/O: `diff(previous, observed) -> Vec<Change>`
   (added, removed, type changed with widened / narrowed / other, primary key
   changed; a rename is a removal and an addition) and
   `decide(policy, changes, source_can_hold_back) -> Decision`. The widening
   list of D3 is one table in this module.
   *Check:* a test per row of the spec's Target table and per policy; a
   property the tests state in words: a breaking change never returns `load`
   for the full column list, under any policy.
5. **API.**
   - `POST /api/connectors/{id}/schema-observations` (the orchestrator's
     service identity only, as `run_failed_event` restricts itself): body
     `{object, columns, primaryKey, phase}` with `phase` `before_load` or
     `after_load`; records, compares, applies the policy, writes the changes,
     calls `evaluate_connector_event` with `ConnectorSchemaChange`, sets the
     pause, and answers the decision. A first observation of a table is the
     baseline and reports no change.
   - `GET /api/connectors/{id}/schema-changes`: waiting changes first, then
     the most recent, bounded.
   - `POST /api/connectors/{id}/schema-changes/approve` with `{object}`:
     approves what waits for that table, accepts the observed columns as the
     new baseline, marks removed columns inactive, lifts the connector pause
     when nothing else waits.
   - `PATCH /api/connectors/{id}` accepts `schemaChangePolicy`.
   - `GET /api/connectors/ingestible` leaves out a paused connector from the
     due list; `POST …/ingest/run` answers 409 with a fixed message for one.
   Every route in `POLICY_TABLE` and asserted both ways in
   `tests/route_auth.rs`; tenant gate on the three user routes
   (`require_connector_in_tenants`); alert text built from names, never from
   source error text.
   *Check:* route tests for each acceptance row that the API alone decides,
   for the tenant gate, and for a user calling the observation route (403).
6. **Orchestrator: observe before loading (batch SQL).** In
   `ingest_factory.py` and the SQL adapters: reflect the table's columns and
   primary key, post them, obey the answer. `wait` records `waiting` and
   returns without loading; `load` with a column list loads only those.
   `EXPECTED_SCHEMAS` in `bronze_catalog.py` stays the single owner of the
   `ingest_run` schema if `waiting` needs a change there. An unreachable API
   fails the step with `Failure`, not retried forever.
   *Check:* unit tests with a fake API and a fake source, no network; test
   names are full sentences.
7. **Orchestrator: observe after loading (other batch sources).** For files,
   REST, MongoDB, Kafka and SFTP: after the load, post the columns the
   pipeline now has, with `phase: after_load`. The API lists added and changed
   columns and never asks such a source to hold anything back.
   *Check:* unit tests; one states that a missing column is not reported as
   removed for these sources.
8. **Orchestrator: new tables under "apply all" (D2).** For PostgreSQL,
   MySQL, MariaDB and SQL Server, when the connector's policy is `apply_all`:
   list the tables of each schema the connector already loads from, and post
   the new ones to `POST /api/connectors/{id}/schema-observations` as a
   `new_table` observation; the API adds them to the connector's selected
   tables with the default load mode and a target derived by the rule the
   console's table picker uses (find it; do not write a second), refusing a
   target that is taken or reserved for uploads, and lists each as a change.
   *Check:* unit tests on both sides; a taken target is listed as "not added"
   with the reason.
9. **A waiting table is not a failure (D7).** The run-event handling of
   `SRC-7` (`routes/load_alerts.rs`) is unchanged for failed and successful
   runs; this task proves with a test that a run whose only unloaded table is
   `waiting` ends as a success in the orchestrator and so raises no failure
   alert and no streak.
10. **Console: connector page.** A "Schema changes" card: what waits, with an
    Approve button per table, then recent changes; the policy select with the
    four choices and one line under each saying what it does (D1), and the
    note of D2 where it applies; a "paused" state on the connector and on a
    waiting table in the ingest panel. Contracts, client, service, feature,
    in that direction.
    *Check:* component tests for each state, including the 403 message.
11. **Console: inactive columns (D8).** The Bronze table's Schema tab
    (`src/features/catalog/asset-columns.tsx`) shows "inactive since <date>"
    for a column the connector marked. Find how that tab gets its columns and
    add the fact to that response; do not add a second request per column.
    *Check:* component test.
12. **Gate.** `ops/g6/g6_ingest_matrix_test.py`, on the MySQL fixture: load;
    add a column and load (column present, change listed); drop a column and
    load (table waits, others load, run not failed); approve (loads, column
    still there). Each step prints what it saw.
13. **Docs (planner).** `docs/plans/SRC-8-RESULT.md`, `CHANGELOG.md`,
    `docs/OPERATIONS.md`, the spec's status.

## 4. Assistant

`Connector` gains three fields; the tool schema snapshot holds input schemas
only (`SRC-7` handoff), so it should not change. If it does, the developer
reports it and the AI team reviews it. No tool is added.

## 5. Build limits on this machine

Rust as before: shared target dir
`/home/hv/.cache/lakehouse-src6-target`, `CARGO_BUILD_JOBS=2`, `df -h /`
before each cargo command, no `cargo test`, no `cargo clean`, no docker.
Python unit tests are run for real in the environment of task 1. Every Rust
test and the gate are *not verified* until CI.

## 6. Out of scope

- Backfilling an added column (spec). CDC connectors (`SRC-4`).
- Per-table column choice by the user (`SRC-11`); only the mechanism is
  shared.
- Discovery for more source types (`SRC-13`); "apply all" lists tables only
  where the orchestrator already can.
- Dropping or altering anything in a Bronze table.

## Handoff

*(developer)*

### Tasks 1-5 (developer, 2026-10-09)

**Commits** (`git log --oneline 9088b69..HEAD`): `01c43e8` task 2 (measurement
test), `34691f1` task 3 (store), `f1acdd1` task 4 (pure comparison and policy),
`3ca9a05` task 5 (API, one commit: the three routes share one file).

**Task 1, environment.** `~/.cache/rantai-dagster-venv`, Python 3.12.3 (CI
uses 3.11; no wheel failed on 3.12). `python3 -m venv` failed (`ensurepip`
missing, no sudo), so: `python3 -m venv --without-pip ~/.cache/rantai-dagster-venv`
then `python3 -m pip --python ~/.cache/rantai-dagster-venv/bin/python install ./dagster pytest==8.3.4`
from the worktree root (user pip 26.0.1). Installed `dlt 1.30.0`, `dagster 1.13.20`,
`pyiceberg 0.12.0`, `pytest 8.3.4`. `duckdb` is not installed and was not
needed. `dagster/build/` and `dagster/dispar_orchestrate.egg-info/` were
created by the install and deleted. Suite before my test file:
`603 passed` (13 s). After: `615 passed` (43 s). No pre-existing failure.
Note found on the way: `sink._install_catalog_env` sets `ICEBERG_CATALOG__*`
in `os.environ` for the whole process and other test modules leave them, so
my new test deletes them with `monkeypatch` (without that, 12 of my tests
failed in the full run and passed alone).

**Task 2, measurement** (`dagster/dispar_orchestrate/test_schema_drift_loader.py`,
12 tests, dlt 1.30.0, `filesystem` destination on a temporary local directory
with `table_format="iceberg"`; the column list is also read back from the
Iceberg metadata with `pyiceberg`). Not the production path: no S3, no
Lakekeeper (dlt's own local catalog), no day partition, no `_ingested_at`,
rows are dicts. Identical under `append` and `replace`:

| Run 1 then run 2 | dlt schema after | Iceberg after |
| --- | --- | --- |
| (a) new column `c` | `a, b, c` bigint | `a, b, c` long |
| (b) `b` stops arriving | `a, b` (`b` stays) | `a, b` long |
| (c) ints, then non-numeric text | `a` bigint **and** `a__v_text` text | `a` long, `a__v_text` string |
| (d) text, then ints | `a` text (5 stored as text, no second column) | `a` string |
| (e) small ints, then 5,000,000,000 | `a` bigint, one column | `a` long |
| (e') hint bigint(32), then bigint(64) | `a` bigint(64), one column | `a` long |

Agrees with D9 ("a second column beside the old one"): measured in (c). For
(d) and (e) the column holds the new values, so no second column appears. Note
for the notice (D9): the second column is named `<column>__v_text`.

**Task 3, store.** Migration `0063_connector_schema_changes.sql`;
`lakehouse-store/src/schema_change.rs`; `Connector`/`ConnectorRow`/
`CONNECTOR_COLUMNS`/`UpdateConnectorInput` carry the policy and pause; TS
`Connector` gains `schemaChangePolicy`, `pausedReason`, `pausedAt` (+ a
`SchemaChangePolicy` type); fixtures fixed. Choices: (1) inactive columns are a
table, not a view over approved rows (the Schema tab asks per table, and a
returning column is one DELETE; reason in the migration header);
(2) `connector_source_schema` also keeps `waiting_columns`/`waiting_primary_key`,
the last OBSERVED shape of a waiting table (the plan said to add it);
(3) `column_name` is `NOT NULL DEFAULT ''` so the pending-identity unique index
`(connector, table, kind, column) WHERE status='pending'` needs no NULL rules;
(4) a pending change the source stops showing is deleted (withdrawn): `status`
has no value for it; (5) approval lifts the pause only when `paused_reason` is
the schema-change text, so a pause set by `SRC-11` is not cleared by it;
(6) `delete_connector` does not clean the new tables: `ON DELETE CASCADE`,
like `connector_probe_result` (0044). `IngestibleConnector` gains `paused`
(serialised); `is_due` leaves a paused connector out, the unfiltered list keeps
it.

**Task 4, pure.** `lakehouse-store/src/schema_diff.rs` (beside the types it
uses and turns into `NewChange` rows). `diff`, `classify_type`, `decide`,
`evaluate`. Interpretations to confirm: (1) a breaking and a non-breaking
change together make the whole table wait and ALL changes `pending` (approval
accepts the observed shape as a whole); (2) the primary key is compared as a
set (a reordered key is not a change); (3) `char(n)` to `varchar(n)` of the
same length is `Other` (breaking); (4) `ask_first` loads the previously
accepted columns still present, a widened column included; (5) integer display
width (`int(11)`) is ignored; `int unsigned` and arrays are opaque (equal
spelling or `Other`); (6) `evaluate()` is `diff` then `decide`; `decide` takes
`(policy, can_hold_back, previous, observed, &changes)`, not the three
arguments the plan names, because `ask_first` needs the shapes.

**Task 5, API.** `routes/schema_changes.rs`. Observation route policy is
`ingest:read` (the ingest identity holds only that, `main.rs`
`bootstrap_ingest_run_service`), plus the `run_failed_event` handler check
(service or `*:*`). The route is registered OUTSIDE
`require_connector_in_tenants` (a service identity has no tenants; the gate
would refuse it); the other two are inside. After-load phase (D4): previous
columns missing from the batch are carried over (not removed, not dropped from
the baseline) and the previous key is kept. `connectors.rs` gained
`pub(super)` on `pool`, `parse_body`, `connector_audit_event` and
`load_alerts::deliver_event` (reused, not rewritten). `ingest/run` checks the
pause BEFORE the spec read (unknown id stays 404).

**Commands and results** (all foreground, from `/home/hv/lakehouse-src6`):
- `df -h /` before each cargo command: 54 GB free throughout.
- `cd rust && cargo fmt --check`: clean.
- `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2 cargo clippy -p lakehouse-store --all-targets -- -D warnings`: clean (after fixing 4 lints in `schema_diff.rs`).
- Same for `-p lakehouse-api`: clean.
- `... cargo clippy --workspace --all-targets --all-features -- -D warnings`: clean.
- `bun run typecheck`: no errors. `bun run lint`: 0 errors, 6 warnings (none in files I touched). `bun run test`: `911 pass, 1 skip, 0 fail` (912 tests, 101 files).
- `cd dagster && ~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate -q`: `615 passed`.
- `python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py`: both OK.

**Existing assertions changed:** none. Existing code touched: the two `Connector`
struct fixtures and the `ConnectorRow` Debug test in `lakehouse-store/src/connectors.rs`,
and the `ingestible()` helper in `routes/connectors.rs` (new fields only).

**Not verified (no `cargo test` on this machine):** every Rust test. New:
`lakehouse-store/tests/schema_change.rs` (11 `#[sqlx::test]`), the unit tests in
`schema_diff.rs` (~25), the unit tests and 10 `#[sqlx::test]` route tests in
`routes/schema_changes.rs`, `update_input_takes_the_four_schema_change_policies...`
and `is_due_leaves_out_a_paused_connector` in `routes/connectors.rs`;
`tests/route_auth.rs` covers the three new `POLICY_TABLE` rows by walking the
table and by the `include_str!` scan of `routes/mod.rs`, also unrun. The
migration itself has never been applied to a database. Also not verified: the
tool-schema snapshot (`bun run test` passed, so it did not change).

### Step 0 and tasks 6-9 (developer, 2026-10-09)

**Commits** (`git log --oneline 3ca9a05..HEAD`): `45d8d28` step 0 (SQL-source
measurement), `31ec837` task 6, `421fde4` task 7, `8dae3fe` task 8,
`01845e8` task 9.

**Step 0, measurement** (`dagster/dispar_orchestrate/test_schema_drift_sql_source.py`,
18 tests; dlt 1.30.0, SQLAlchemy 2.0.36, SQLite file through
`sql_database(credentials=..., table_names=[...])` as `adapters/sql.py` builds
it, `reflection_level="full"`, same local Iceberg destination as task 2). Run 1,
then the table is dropped and recreated with the new definition, run 2.
Identical under `append` and `replace`. SQLite has type affinity, not strict
types: the docstring says what that does and does not tell about PostgreSQL,
MySQL, SQL Server and Oracle (the reflected-type-to-dlt-type step and what
dlt/Iceberg do with a changed hint carry over; dialect spellings, precision and
server-side enforcement do not).

| Source change | Run 2 | dlt schema after | Iceberg after |
| --- | --- | --- | --- |
| (a) column `c` added | loads | `a, b, c` | `a, b, c` |
| (b) column `b` dropped | loads | `a, b` (`b` stays) | `a, b` |
| (c) `INTEGER` -> `TEXT`, non-numeric values | **load FAILS** (`PipelineStepFailed` / `LoadClientJobRetry`) | `a` retyped to `text`; **no `a__v_text`** | `a` stays `long` (Iceberg refuses `long -> string`) |
| (d) `TEXT` -> `INTEGER` | **load FAILS** (same) | `a` retyped to `bigint` | `a` stays `string` (refuses `string -> long`) |
| (e) `INTEGER` -> `BIGINT`, values past 32 bits | loads | one column `bigint` | one column `long` |
| (e2) `VARCHAR(5)` -> `VARCHAR(500)` | loads | one column `text` | one column `string` |
| (f) primary key `a` -> `b` | loads | `primary_key` flag moves to `b` | no key kept |

Also measured: after (c) fails, the next load fails too, even when the source
column is put back to `INTEGER` (run 3 same definition and run 4 original
definition both fail): the failed package stays pending in the pipeline's
working directory (production: `bronze_ingest_<table>` in the code location
container) and is retried first. So a breaking type change that is allowed to
reach `pipeline.run` wedges the table until that directory is gone. This makes
the before-load observation load-bearing for SQL sources.

**CONTRADICTION with feature-page decision 9 for SQL sources:** (c) does not
produce "a second column beside the old one"; the load fails and the old column
is untouched. The second column `<column>__v_text` (task 2) appears only for
types dlt INFERS from rows (files, REST, MongoDB, Kafka, SFTP). Decision 9's
wording needs a split by source kind (planner).

Column selection (dlt 1.30.0, installed source read): `sql_database` accepts
`table_adapter_callback`, called with each REFLECTED SQLAlchemy `Table` while
the source is BUILT (before any row is read); removing columns from
`table._columns` there keeps them out of the SELECT, the hints and the
destination (tested). `sql_table(..., included_columns=[...])` does the same
for one table. `sql_database` itself has no `included_columns`. Task 6 uses the
callback for both reading (`ReflectionCollector`) and holding back
(`keep_only`).

**Task 6.** `schema_observer.py` (new): `ObserverConfig.from_env()` (copies
`IngestFactoryConfig.from_env()`, function-local import to avoid a cycle; the
bearer header is `ingest_factory._headers`, the one the ingestible GET sends),
`post_observation`, `Decision`, `ReflectionCollector`, `keep_only`, and
`ObservationUnreachable` (connection/timeout/5xx: `ingest_source_object` raises
a retryable `Failure`) / `ObservationRefused` (4xx, unset token, non-decision
answer, no table reflected: `Failure(allow_retries=False)`). `wait`: an
`ingest_run` row with status `waiting` (`status` is a free `String` column, so
`EXPECTED_SCHEMAS` is untouched), nothing loaded, nothing raised, returns
`None`. A column list rebuilds the source once with `keep_only` (second
reflection, same guard; observed once). Run id: `context.run_id` ->
`_run_one_object(..., run_id=)`. How reflection stays inside the SSRF guard:
postgres - `run_bronze_ingest(gate=)` reflects inside `sql_database(...)` with
`engine_kwargs={"connect_args": {"hostaddr": <checked ip>}}` (reaches the
reflection engine too); generic SQL (mysql/mariadb/mssql) - mssql reflects over
the pinned ODBC string, **mysql/mariadb reflection was NOT pinned before this
work** (`build_source` called `sql_database` before the caller's
`pinned_resolution` around the load) and is now wrapped in
`ssrf_guard.pinned_resolution` inside `build_source`; oracle - reflection runs
inside `build_source`'s `checking_resolver()` over the checked IP literal. No
connection of the observer's own exists. `column_gate.reject_unsupported_column_types`
still runs first and is unchanged.

**Task 7.** `SinkResult.columns` (`LoadedColumn(name, data_type, nullable)`),
read by `sink._loaded_columns` from the pipeline's schemas, without `_dlt_*`
and `_ingested_at` (not columns of the source; `<column>__v_<type>` variants
ARE reported, they are real Bronze columns). Posted with `phase: after_load`,
`primaryKey: []` after the load is recorded, for files/rest/mongodb/sftp
(generic arm, `adapter != "sql"`) and the Kafka micro-batch (after commit);
`sheets` and SQL are not posted after. A failed post is a logged warning
(exception type, or the observer's fixed message), never a failed run; the
comment at `_observe_after_load` says why this is the one place fail-closed
does not apply.

**Task 8.** *API* `POST /api/connectors/{id}/schema-observations/tables`,
`routes/schema_changes.rs::new_tables`. Request `{"tables": ["schema.table", ...],
"runId"?: string}`, 1-2000 names (blank/oversize/control-character names and
unknown fields are 400; repeats dropped). Response
`{"added": ["schema.table", ...], "notAdded": [{"table": "schema.table", "reason": "<fixed text>"}]}`.
Errors: 403 (not the service identity / administrator), 404 unknown connector,
409 fixed texts when the policy is not `apply_all` or the source is not an
`sql` adapter with driver postgres/mysql/mssql (`mariadb` is `mysql` in the
dial; Oracle is not listed). Gate: `ingest:read` + handler check (shared
`require_orchestrator`), outside the tenant gate, one `POLICY_TABLE` row.
Target rule: `lakehouse_store::ingest_spec::default_bronze_target`, a port of
`defaultTarget` in `connector-ingest-panel.tsx` (comments in both; both tests
pin the three examples plus four edge cases). A name the connector already
selects is skipped (in neither list: a second identical post answers
`{"added": [], "notAdded": []}`). Refused (a `table_added` change `pending`,
`afterValue` = the fixed reason, not appended): target used by any connector
(`any_connector_targets`, which includes this connector's own other tables),
target in `upload_table_claim` (`uploads::table_claimed`, the primitive behind
`refuse_uploaded_targets`), or a target another table of the same request took
(decided under the row lock). Added: `source_objects` gains
`{name, target, loadMode: "replace"}` through `save_ingest_spec_in` (the same
`UPDATE` and checks `set_ingest_spec` runs, dial and adapter written back
unchanged so `is_repoint` is false; `save_ingest_spec` was split, no
behaviour change), change `applied` with `afterValue` = the target. One
`ConnectorSchemaChange` alert per batch, only when a change is new.
`IngestibleConnector` gained `schemaChangePolicy` (store + serialised, one new
test). *Orchestrator* `_discover_new_tables` in `run_ingest`, before the
fan-out: lists the base tables of every schema already loaded
(`adapters/sql.py::list_tables`, guarded like the loads), posts unselected ones
in chunks of 2000, and **re-reads the connector so the new tables load in the
same run**. A failure there is a warning, never a failed run (nothing loads
unchecked because of it; the already selected tables must keep loading).

**Task 9.** No behaviour change; proof only. Python test through the real job
graph (one table `wait`, one loads: all steps succeed, `waiting`/`succeeded`
rows, no materialization for the waiting one). Rust test in `load_alerts.rs`: a
SUCCESS run event with a pending schema change delivers `connector_success`
only, no failure/repeated rules, streak 0, a `success` event row, change still
pending.

**Commands and results** (all foreground, from `/home/hv/lakehouse-src6`):
- `cd dagster && ~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate -q`:
  after step 0 `633 passed`; task 6 `683 passed`; task 7 `694 passed`; task 8
  `727 passed`; task 9 `728 passed` (121 s each full run).
- `python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py`: both OK after each commit.
- `df -h /` before each cargo command: 48 GB free.
- `cd rust && cargo fmt --check`: clean (after `cargo fmt`, which only touched my hunks: three files).
- `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2 cargo clippy -p lakehouse-store --all-targets -- -D warnings` and `-p lakehouse-api`: clean.
- `... cargo clippy --workspace --all-targets --all-features -- -D warnings`: clean (final tree).
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (none in files I touched). `bun run test`: `912 pass, 1 skip, 0 fail` (913 tests, 101 files).

**Existing tests changed (fakes only, no assertion weakened or removed):**
`test_ingest_factory.py`: fake adapters now take `table_adapter_callback=None`
and reflect (`_reflect`) because the SQL arm reflects before it loads (the
resolves-secrets, records-failed, oracle-routing and the batch `_FakeAdapter`
tests); fake `run_bronze_ingest` takes `gate=None` (three places); fake
`_run_one_object` takes `run_id=None` (four places) and fake
`_run_stream_connector` takes `run_id=None` (two places); the batch
`_FakeOutcome` carries `columns = ()`; a new autouse fixture `observations`
answers `load` to every observation. `test_adapters_sql.py` and
`test_adapters_oracle.py`: only added tests. No Rust assertion changed; the
Rust fixture `ingestible()` gained the new field.

**Not verified (no `cargo test` on this machine):** every Rust test. New:
`lakehouse-store/tests/schema_change.rs` (7 `#[sqlx::test]`: appended table
with `replace`, second identical request writes nothing, caller-refused table
waits and is not new the second time, same-connector target taken under the lock
and in-batch, connector without a spec, unknown connector, ingestible carries the
policy); `ingest_spec.rs` (2 unit tests of the target rule);
`routes/schema_changes.rs` (7 `#[sqlx::test]` + 2 unit tests: added with picker
targets and listed applied, taken target by another connector, upload-reserved
target, second identical post and one alert, 409 for other policies, 409 for an
unlisted source, 403/400/404); `routes/load_alerts.rs` (1 `#[sqlx::test]`). The
new `POLICY_TABLE` row is covered by `tests/route_auth.rs` through the table
walk and the `include_str!` scan of `routes/mod.rs`, unrun. Also not verified:
`list_tables` against a real PostgreSQL/MySQL/SQL Server (only fake engines),
`connect_timeout` on a real libpq/pymysql, and the whole observation path against
a real source (the gate, `ops/g6`).

**Plan/code mismatches and notes for the planner:**
1. Decision 9 (above): SQL INTEGER <-> TEXT fails the load rather than adding a
   second column.
2. Task 8 as written said "post to `schema-observations` as a `new_table`
   observation"; the lead's brief replaced it with the `.../tables` route, built.
3. A pending `table_added` change (a table that was NOT added) can be
   "approved" through `POST .../schema-changes/approve`, which only marks it
   approved (it does not add the table). The console (task 10) should not offer
   Approve for it, or the API should refuse; not decided here.
4. A pending `table_added` refusal counts as "something else waits" in
   `approve_object` and so blocks lifting a schema-change pause set by another
   table until it is approved.
5. Name matching for "already selected" is exact: on SQL Server a manual
   `dbo.orders` against the listed `dbo.Orders` reads as a new table whose target
   is then taken, a harmless pending row (one alert) every run.
6. `list_tables` for MySQL treats the schema as the database name (the
   convention `defaultDiscoverSchema` uses).
7. mysql/mariadb reflection was unpinned before (see task 6); fixed and tested.

### Review fixes and tasks 10-12 (developer, third pass, 2026-10-09)

**Commits** (`git log --oneline 541f475..HEAD`): `707deb4` BLOCKER 2 and
SHOULD-FIX 3 together (one commit: both rewrite `approve_object` and share the
`SchemaChange` flag, so they cannot be split by file), `aad9663` task 10,
`4465f2d` task 12, and a one-line lint fix for the task 10 test. **Task 11 was
not built** (see below).

**Inherited work in progress.** Kept: `can_approve` (non-stored), the
`cannot_be_loaded` rule, `ApproveOutcome`, `refusal_dismissed`, the
`kind <> 'table_added'` pause check. Changed: (1) `dismiss_table_added` by
change id is DISCARDED. Dismissal reuses `POST .../schema-changes/approve` with
the table's name (`{object: "<schema.table>"}`): the wire shape is unchanged,
approving a pending `table_added` row marks it `approved` ("seen"), adds
nothing and touches no baseline or pause. (2) `can_approve` for a pending
`table_added` is now `true` (the route takes it); and a blocked table
(a type change the column cannot hold) marks ALL its waiting column-level
changes false, not only the type change, through `mark_approvable`, applied in
`list_pending`, `list_recent`, `ObservationTx::record` (the observation
answer) and per row in `insert_change_row`; approve returns approved rows
(false by status). (3) `approve_object` split (`accept_waiting_shape`) for
`clippy::too_many_lines`. The "applied" path of SHOULD-FIX 3 (d): an added
table is in `source_objects`, so a later request skips it (`AlreadySelected`);
only a refusal (caller's or `TargetTaken` under the lock) is matched against a
dismissed row, and a different reason inserts a new pending row (tests).

**Decisions in the code.** Decision 10: 409 `ApiError::Conflict` with the fixed
text `CANNOT_BE_LOADED`; nothing is written. After-load phase: such a change is
never pending (recorded `applied`), test kept. Withdrawal: integer->text ->
back -> `load`, nothing pending is a store test and a route test.

**Task 10.** New tab "Schema changes" (kept mounted) in
`connector-detail-page.tsx`, component `connector-schema-changes-panel.tsx`;
contracts `SchemaChange`, `SchemaChangeList`, `InactiveColumn`, approve
request/response, `UpdateConnectorInput.schemaChangePolicy`; client and service
`listSchemaChanges`, `approveSchemaChanges`. Policy is a radio group (four
choices, one line each, the decision-2 note under "Apply all" when
`driverForType(type)` is undefined). The page reads the list once (panel) and
hands the waiting set to the Ingest tab (`waitingTables`, "Waiting for a
decision"); the tab carries a count while tables wait. Header and Sources row:
`Paused: <reason>` pill; the Ingest tab's `blockedReason` starts with it and
disables Run now. **`__v_text` finding:** `sink._loaded_columns` reports the
variant column as an ordinary column, so the API records a `column_added` row
named `<column>__v_text` and no `type_changed` row (dlt does not retype the old
column, `SRC-8-RESULT.md`). The data does not say why it appeared, so the
console shows it as "Column added" with no explanatory line.

**Task 11: STOPPED, per the brief.** The Bronze table maps to its connector
object cleanly (catalog `table_name` = `source_objects[].target`, written by
`connector_catalog.register_connector_table`), but the column cannot be matched
reliably: `connector_inactive_column.column_name` is the SOURCE name as the
reflection returned it, the Schema tab's names come from `DESCRIBE TABLE` of the
Bronze table, i.e. dlt's snake_case of the source name (`OrderDate` ->
`order_date`), and nothing in Rust reproduces dlt's normaliser. An exact match
would silently skip every non-snake_case column; a loose match could mark the
wrong one. Proposal (needs a plan decision, wire change): the before-load
observation also carries the dlt-normalised name per column
(`schema.naming.normalize_identifier`, computed where dlt is), stored with the
inactive column, and the catalog detail joins on it.

**Task 12.** `step_source_schema_changes` in `ops/g6/g6_ingest_matrix_test.py`
(new table `src8_drift`, connector `g6-mysql-schema-drift`, target
`g6_mysql_schema_drift`, load mode `replace`, default policy). Asserts: step 1
3 rows and nothing listed; step 2 run success, `note` in the Bronze columns
(polled), `column_added` `applied` in `recent`; step 3 run success, a pending
`column_removed` `qty` with `canApprove` true, an ingest-runs row `waiting`,
Bronze count unchanged; step 4 approve 200, run, the extra row loaded (count
+1), `qty` still a Bronze column, `inactiveColumns` lists it, nothing pending.
Not asserted: the alert, other policies, type/key changes, the Schema tab mark.
No compose or CI change was needed (`g6_reader` owns `g6_ingest` through
`MYSQL_DATABASE`/`MYSQL_USER`).

**Commands and results** (foreground, `/home/hv/lakehouse-src6`; `df -h /` 46 GB
before each cargo run):
- `cd rust && cargo fmt --check`: clean (after `cargo fmt`, which touched only my hunks).
- `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2 cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings` (after `touch` of the changed sources): clean, 2m21s; one earlier run failed on `too_many_lines` in `approve_object`, fixed.
- `... cargo clippy --workspace --all-targets --all-features -- -D warnings`: clean.
- `python3 -m py_compile ops/g6/g6_ingest_matrix_test.py`; `check_intra_package_imports.py`; `check_bare_iceberg_count.py`: OK. Nothing under `dagster/` changed, so the Dagster suite was not re-run (728 passed at `01845e8`).
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (the same six as before, none mine). `bun run test`: `927 pass, 1 skip, 0 fail` (928 tests, 102 files; 912 pass before, +15).

**Assertions changed:** `connector-detail-page.test.tsx` "opens on Overview,
with the address left clean": the tab list had three entries, now four (Schema
changes between Ingest and Connection tests); its fetch stub gained the new GET.
No Rust assertion changed: the store tests' `approve_object(...)` calls gained
`.into_approval()` because the return type is now `ApproveOutcome`.

**Not verified (no `cargo test`, no docker):** every Rust test, new ones: store
`tests/schema_change.rs` (6: cannot-be-loaded refusal and sibling block, put
back withdraws, narrowed still approvable, notice does not keep a pause,
dismissal and different reason, added table never recorded again); routes
`schema_changes.rs` (5: 409 under four policies with exact text and nothing
approved, put back loads, after-load applied, removal approvable flag, dismiss
through approve with one alert); the migration against a database; the gate
(first run is CI); the Dagster suite after this pass (no Python changed).
Known gap found, not fixed (outside the brief): a connector paused by policy
`pause` because of a type change the column cannot hold is not observed again
(a paused connector does not run), so "change the column back at the source"
cannot lift the pause by itself; the pending row would only be withdrawn by an
observation. Needs a planner decision.

### Fourth pass: review BLOCKER 3, task 11, gate (developer, 2026-10-09)

**Commits** (`git log --oneline 9995735..HEAD`): `0363fda` BLOCKER 3a, `dad96dc`
BLOCKER 3b, `6d61f2d` task 11 (orchestrator, API, catalog, console in one
commit: the wire change spans all three), `b200550` gate.

**BLOCKER 3a.** `schema_diff::decide`: in the breaking branch,
`pause_connector` is `policy == Pause && !cannot_be_approved`, where
`cannot_be_approved` is "the changes include a `TypeChanged` classified
`Other`" (the same classification `SchemaChange::cannot_be_loaded` uses). The
after-load branch (`!can_hold_back`) is unchanged: such a change is recorded
`applied` and never waits, so nothing can be stuck there. Tests: two pure
cases; route test
`under_pause_an_unloadable_type_change_waits_without_pausing_and_clears_when_put_back`.

**BLOCKER 3b.** The one write of the selection is
`connectors::save_ingest_spec_in` (a person's save, a re-point and "apply
all" all pass through it). It now reads the stored `source_objects` under the
row lock it already takes and, after the `UPDATE`, calls
`schema_change::clear_removed_tables_in` for the names that left the list:
deletes their pending changes, `connector_source_schema` row (waiting shape
and baseline) and inactive columns, then lifts the pause through
`lift_pause_if_nothing_waits`, which was cut out of `approve_object` (one
copy, used by both; only the schema-change reason is lifted). History rows
(`applied`, `approved`) stay. The console constant and the route's
`CANNOT_BE_LOADED` already both said "remove the table from the connector and
add it again under a new target"; not touched. Tests (store, three): removal
clears everything and lifts the pause and re-adding is a baseline
observation; removal of one table leaves the other's pending change and the
pause; a pause with another reason stays; plus the pure
`removed_object_names`.

**Task 11.** Loaded name: `schema_observer.loaded_column_name` =
`dlt.Schema("bronze").naming.normalize_identifier(name)`, `None` if dlt
raises. `ReflectionCollector` puts it on each before-load column
(`loadedName` on the wire only when not `None`); after-load columns send
their own name. `test_schema_loaded_name.py` loads a SQLite table with
`OrderDate`, `Ship Date`, `already_snake`, `Qty`, `2nd`, `a-b` through the
drift tests' local Iceberg destination and asserts that the set of computed
names equals the Iceberg column names (`order_date`, `ship_date`,
`already_snake`, `qty`, `_2nd`, `a_b`). API: `ObservedColumn.loaded_name`
(`#[serde(default, skip_serializing_if)]`, validated like a column name);
migration `0064_connector_inactive_column_loaded_name.sql` (`loaded_name`
nullable); `accept_waiting_shape` reads the removed column's loaded name from
the OLD baseline (it now runs the inactive inserts before the baseline moves);
`inactiveColumns[].loadedName`. Catalog: `bronze_asset_detail_body` sets
`inactiveSince` (null by default) on each column and
`mark_inactive_columns` fills it from ONE query,
`schema_change::inactive_columns_of_table(pool, table)`: joins
`connector_inactive_column` to the connector's `source_objects[].target` =
the Bronze table name and `.name` = `object_name`, `loaded_name` equal to the
column's name; NULL `loaded_name` is left out. It runs after
`catalog_tenant_refusal` in `detail` and takes only the table name; no pool
or a failed query logs a warning and leaves the detail unmarked. Console:
pill "inactive since <date>" in `Marks`, title as asked; `AssetColumn.inactiveSince?: string | null`
(optional because Silver and Serving details do not carry it),
`InactiveColumn.loadedName: string | null`.

**Gate.** Step 4 of `step_source_schema_changes` polls
`GET /api/catalog/g6-mysql-schema-drift` (the gate's session is the bootstrap
admin; the catalog tenant gate never refuses it, so nothing is skipped) until
the schema lists `qty`, then asserts `qty.inactiveSince` is set and no other
column has one; prints a `[g6] SRC-8 step 4: catalog detail ...` line. If the
catalog registration (best effort) never happened it fails and says so.

**Commands and results** (foreground, `/home/hv/lakehouse-src6`; `df -h /` 46 GB
before each cargo run):
- `cd rust && cargo fmt --check`: clean (after `cargo fmt`, which touched only my hunks).
- `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2 cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings` after `touch` of the changed sources, per commit: clean (1m 21s, 1m 35s, 1m 41s); one run failed on `explicit_auto_deref`, fixed.
- `... cargo clippy --workspace --all-targets --all-features -- -D warnings`: clean (1m 25s).
- `cd dagster && ~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate -q`: `731 passed` (3 new in `test_schema_loaded_name.py`), then `733 passed` after two more tests in `test_ingest_factory.py` / `test_schema_observer.py` (those two files alone: `118 passed`; the full suite was not re-run after the last two tests).
- `python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py`: both OK. `python3 -m py_compile ops/g6/g6_ingest_matrix_test.py`: OK.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (the same six, none mine). `bun run test`: `928 pass, 1 skip, 0 fail` (929 tests, 102 files; 927 before, +1).

**Assertions changed:** none removed or weakened. One test fixture gained a
field: `connector-schema-changes-panel.test.tsx` `inactiveColumns` row now has
`loadedName` (the contract requires it). Struct literals of `ObservedColumn`
in tests gained `loaded_name: None`.

**Not verified (no `cargo test`, no docker):** every Rust test, new ones: pure
(`schema_diff.rs`, 2), route (`schema_changes.rs`: pause without stuck
connector, loaded name kept on approval, loadedName validation),
`catalog.rs` (`inactive_marks`, 2: marks by loaded name and a failed lookup),
store (`tests/schema_change.rs`: 3 for removal, 1 for
`inactive_columns_of_table`; `connectors.rs`: `removed_object_names`); migration
0064 never applied to a database; the gate (first run is CI) including whether
the catalog registration happens in the gate stack. The Bronze catalog detail
route as a whole is not covered by a route test (it needs ClickHouse mocks for
its eight queries); only `mark_inactive_columns` is, with a real pool.
The loaded name is computed on a default `dlt.Schema`, not on the pipeline's
own schema, so a name past dlt's default identifier length could differ from
the destination's (limit written in the function's docstring).

**Notes for the planner.** Migration 0064 is the next free number on this
branch; `main` has its own `0061`/`0062`-range files, so numbers must be
reconciled when the branch meets `main`. Existing inactive rows (approved
before this pass) have NULL `loaded_name` and mark nothing.

## Review

### Tasks 1–5, 2026-10-09, at `3ca9a05`

Read migration `0063`, the route outline and the developer's fourteen
interpretations. No `BLOCKER`.

Re-run by the planner on `3ca9a05`: `cargo fmt --check` clean;
`cargo clippy --workspace --all-targets --all-features -- -D warnings` no
warnings; the Dagster suite in `~/.cache/rantai-dagster-venv` (Python 3.12;
CI uses 3.11) `615 passed`, which is also the first real run of `SRC-7`'s
sensor tests; `check_intra_package_imports.py` passes; `bun run typecheck`
clean; `bun run lint` 0 errors, 6 warnings in untouched files;
`bun run test` 911 pass, 1 skip, 0 fail. No Rust test was run by anyone.

Findings:

- `SHOULD-FIX` 1 (planner's gap, for the next pass). Task 2 measured rows
  whose types the loader infers from the data. The sources decision D4 checks
  before loading are SQL tables, where the type comes from the database's
  own column definition. What the loader does when that definition changes
  (integer to text, text to integer, a larger integer, a dropped column) is
  not measured, and D9's wording rests on it. Measure it with a SQL source
  before task 6 is wired.
- Accepted: a breaking and a non-breaking change seen together make the
  whole table wait; a pending change the source stops showing is removed; a
  reordered primary key is no change; the same length under `char` and
  `varchar` is treated as breaking (fail closed).
- Accepted with a note: the observation route sits under `ingest:read`, the
  only permission the orchestrator's identity has, and the handler admits
  only a service identity or an administrator. A read permission guarding a
  write is odd; a dedicated permission needs a scope change for identities
  that already exist, so it is a backlog item, not this work.
- For the feature page: when a type widens in a way the column cannot hold,
  the second column is named `<column>__v_text` (measured, task 2).

### Step 0 and tasks 6–9, 2026-10-09, at `01845e8`

Re-run by the planner on `01845e8`: `cargo fmt --check` clean;
`cargo clippy --workspace --all-targets --all-features -- -D warnings` no
warnings; Dagster suite `728 passed`; both `ops/lint` scripts pass;
`bun run typecheck` clean; `bun run test` 912 pass, 1 skip, 0 fail. No Rust
test was run by anyone.

The measurement is written up in `docs/plans/SRC-8-RESULT.md`. It overturned
feature-page decisions 3 and 9; both are reworded there and wait for the
product owner, with decision 10 (new).

Findings:

- `BLOCKER` 1. `schema_diff.rs` classifies "anything to text" and a decimal
  with more digits as widened, so under the two "apply" policies the API
  answers `load` for a SQL table whose column went from integer to text. The
  measurement shows that load fails and leaves the table failing for good.
  Widened is now: a larger integer, float to double, a longer text. Every
  other change of type is `Other`, which is breaking.
- `BLOCKER` 2. Approving such a change resumes the table, and the next load
  fails the same way. Approval must be refused (409, fixed message, decision
  10) when what waits for the table includes a type change classified
  `Other` from a source that is checked before loading. The table leaves the
  waiting state when the source no longer shows the change.
- `SHOULD-FIX` 3. A table that "apply all" could not add is stored as a
  pending change. It must not count as "this table waits", must not keep a
  connector paused, and, once a person has dismissed it, must not come back
  as a new row and a new alert on every run.
- Accepted, and a real fix to code that predates this work: reflection for
  MySQL and MariaDB ran outside the address pin; `build_source` now pins it.
- Accepted: new tables load in the run that found them; the tables route is
  separate from the column route.
- Limit to write down: table names are compared exactly, so on SQL Server a
  table selected as `dbo.orders` and listed as `dbo.Orders` reads as new.


### Review fixes and tasks 10, 12, 2026-10-09, at `9995735`

Re-run by the planner on `9995735`, after touching the changed Rust sources so
the crates really recompiled (1m 20s): `cargo fmt --check` clean;
`cargo clippy --workspace --all-targets --all-features -- -D warnings` no
warnings; Dagster suite `728 passed`; `py_compile` of the gate script and
`check_intra_package_imports.py` pass; `bun run typecheck` clean;
`bun run lint` 0 errors, 6 warnings in untouched files; `bun run test` 927
pass, 1 skip, 0 fail, 928 tests in 102 files. No Rust test was run by anyone.

`BLOCKER` 2 and `SHOULD-FIX` 3 are fixed in `707deb4`.

Findings:

- `BLOCKER` 3 (found by the developer, a hole in the planner's design). With
  policy "pause", a type change the column cannot hold pauses the connector.
  A paused connector does not run, so it is never observed again; "change the
  column back at the source" can then never clear the change, and Approve is
  refused. The connector is stuck. Two rules close it:
  (a) a change that cannot be approved never pauses the connector: its table
  waits, the rest keeps running, so a later run sees the column put back;
  (b) when a table is removed from a connector's selection, what waits for
  that table is cleared, and a schema-change pause is lifted when nothing
  else waits.
- Task 11 was stopped correctly. The mark must be matched on the name the
  column has in the Bronze table, which is the loader's normalised name, and
  nothing on the API side knows that name. Planner decision: the orchestrator
  sends it. Each observed column carries `loadedName`, the name the loader
  gives it; the API stores it with an inactive column; the catalog detail
  matches on it. A column with no `loadedName` gets no mark.
- Accepted: the console shows a second column `<column>__v_text` as a plain
  "Column added", because nothing in the data says why it appeared.

### Fourth pass, 2026-10-09, at `b200550`

Re-run by the planner on `b200550`, after touching the changed Rust sources
(recompiled, 1m 23s): `cargo fmt --check` clean;
`cargo clippy --workspace --all-targets --all-features -- -D warnings` no
warnings; Dagster suite `733 passed`; both `ops/lint` scripts and
`py_compile` of the gate script pass; `bun run typecheck` clean;
`bun run lint` 0 errors, 6 warnings in untouched files; `bun run test` 928
pass, 1 skip, 0 fail, 929 tests in 102 files.

`BLOCKER` 3 is fixed in `0363fda` and `dad96dc`; task 11 is built in
`6d61f2d`; the gate checks the mark in `b200550`. No open `BLOCKER` or
`SHOULD-FIX`.

Not verified, by anyone:

- Every Rust test of `SRC-8`, and migrations `0063` and `0064` on PostgreSQL.
- The g6 step. It is the first run of this work against a real database
  (MySQL); the measurement in `docs/plans/SRC-8-RESULT.md` used SQLite.
- The catalog detail route as a whole has no route test; only the helper
  that adds the marks is tested.
- Nothing was opened in a browser.

Open before this branch can meet `main`:

- `main` now has `0061_semantic_entry_roles.sql`. This branch's `0061` to
  `0064` have never been applied to any database, so they are renumbered
  when `main` is merged in (a developer task: the files, and every comment
  and test that names a number).
- Feature-page decisions 3 and 9 (reworded after the measurement) and 10
  (new) were signed by the product owner on 2026-10-10, as built. The
  second-column alternative is backlog `SRC-15`.

Limits found while building, for the feature page:

- The loaded name of a column is computed with the loader's default naming
  rules; a column name longer than the loader's default limit could be
  matched wrongly and then gets no mark.
- A column marked inactive before the loaded name existed gets no mark in the
  Schema tab. No such rows exist outside this branch.
- Table names are compared exactly: on SQL Server `dbo.orders` and
  `dbo.Orders` read as two tables.
