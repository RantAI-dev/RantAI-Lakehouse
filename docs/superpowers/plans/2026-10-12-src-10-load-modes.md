# `SRC-10` More load modes — Implementation Plan

**Status:** decisions signed 2026-10-12; not started: waits for PR #101
(`SRC-6`, `SRC-7`, `SRC-8`) to merge. The product owner chose the spec for
history (SCD type 2) over the planner's proposal to defer it, so this plan
builds it. Decision 9 on the feature page is new and unsigned; its default is
built.
Written 2026-10-12 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/src-10.md`. Backlog `SRC-10`.
**Feature page:** `docs/core/features/load-modes.md`. Decision numbers below
(D1–D9) are its rows.

**Base and branch:** `main` after PR #101 is merged; branch
`feat/src-10-load-modes`, opened as a draft pull request from its first
commit so CI proves the gate step. Built in parallel with `SRC-9`
(`feat/src-9-connector-gates`); section 5 says how the two stay apart.

**Why.** A table can be replaced, appended to, or loaded incrementally. It
cannot keep one current row per key, and one table cannot be refreshed or
cleared by itself.

---

## 1. What exists today (anchors verified at `8973132`, the PR #101 branch)

| # | Spec row | Code | Verdict |
| --- | --- | --- | --- |
| F1 | Sync modes: "append and replace" | `lakehouse-store/src/ingest_spec.rs` `LoadMode` is `Replace`, `Append`, `Incremental`; `validate_load_modes` allows `Incremental` for the `sql` adapter only. `dagster/dispar_orchestrate/adapters/sink.py` `LOAD_MODES = ("replace", "append", "incremental")`. | The spec's "Today" is stale: three of Airbyte's five modes exist. Corrected by name in task 9. |
| F2 | Deduplicate on primary key: none | `SourceObject` has `name`, `target`, `load_mode`, `incremental_key` and no key. `sink.py` maps every mode to dlt `append` or `replace`. | Confirmed. |
| F3 | History: none | No code. Measured 2026-10-12 with the shipped loader (dlt 1.30.0, pyiceberg 0.12.0, local Iceberg table): `merge` with strategy `scd2` is refused, "Available strategies: ['upsert', 'insert-only']". | Cannot be met with the loader alone. Built on top of the merge the loader does have. D1. |
| F4 | Refresh one table, or clear it: none | `routes/connectors.rs` `ingest_run` sends `{"ops": {"run_ingest": {"config": {"connector_id": id}}}}`; `ingest_factory.py` `run_ingest` fans out over every source object. | Confirmed. |
| F5 | *Not in the spec* | Measured the same day: `merge` with strategy `upsert` and a primary key leaves one current row per key across two loads. A batch that holds one key twice fails the load (`LoadClientJobRetry`), with or without a `dedup_sort` hint. A `hard_delete` column fails with a schema mismatch. | Dedup before loading is ours to do (D4). No delete handling. |
| F6 | *Not in the spec* | `SRC-8` already reads a SQL table's primary key before loading (`schema_observer`, `ReflectedColumn`) and makes the table wait when it changes (`schema_diff.rs`). `SRC-8` also proved column selection through `table_adapter_callback`. | The key "detected" by the spec is already in hand for SQL sources. |
| F7 | *Not in the spec* | The measurement used rows as dicts on a local filesystem catalog. It did not use a SQL source, Lakekeeper, the day partition on `_ingested_at`, or the query engine. | Task 1 repeats it on the product's path before anything is wired; the gate proves the query engine (D8). |

## 2. Decisions already made by the planner

- **Three new mode values**, `replace_dedup`, `incremental_dedup` and
  `history`, beside the three that exist.
- **History is our own merge on top of the loader's `upsert` (D1).** A
  history table has three more columns, `_valid_from`, `_valid_to` and
  `_is_current`, and a `_row_hash` of the source columns that are not the
  key. Its merge key is the table's key plus `_valid_from`. For each load:
  read the current row of every key in the load; a key whose hash differs
  gets its current row written again with `_valid_to` set and `_is_current`
  false, and a new row with `_valid_from` = the load's time; a new key gets
  one row; an unchanged key gets nothing. Everything goes in one `upsert`,
  which is one commit of the table. A key missing from the load is left
  alone (D9).
- **History is allowed only if task 1 proves three things**: the load is one
  commit, a second identical load writes nothing, and a load that fails
  half-way leaves no closed row without its successor. If one fails, stop.
- **The key lives on the table entry**: `SourceObject.primary_key`, a list of
  column names. The console fills it from the source's own key where `SRC-8`
  reads one.
- **Deduplication happens in the loader before the merge**, on the key, by
  the cursor column (D4), because the loader fails on a repeated key (F5).
- **Refresh and clear are a run of one table with an action**, not new
  machinery: the run's config names the table and one of `refresh_keep`,
  `refresh_remove`, `clear`. A normal run names neither and behaves as today.
- **`_ingested_at` and the day partition stay.** A merged row carries the
  time of the load that last wrote it. Task 1 measures what a merge does to
  the partition spec; if it cannot be kept, stop and report.

## 2a. Spec row to tasks

| Spec Target row | Tasks |
| --- | --- |
| Sync modes, five, per table | 2, 3, 4, 6 |
| Deduplicate on a declared or detected primary key with a cursor column | 1, 2, 3, 4, 6 |
| History (SCD type 2) | 1, 2, 3a, 4, 6, 8 |
| Refresh one table, or clear it | 5, 7 |

## 3. Tasks

One task per commit. Python tests run for real in
`~/.cache/rantai-dagster-venv`.

1. **Measure on the product's path (F7).** A unit test file, like
   `test_schema_drift_loader.py`: through `sink.load_via_sink` itself where
   it can run locally, otherwise the same calls with the same hints
   (`_ingested_at` stamp, day partition, format version 2): merge on a key
   over two loads; a repeated key in one batch after our own dedup; a
   composite key; a key column that is null; a merge after a `replace`; an
   incremental merge with a cursor; a merge into a table first loaded with
   `append` (existing duplicates). For history (D1): two loads with one
   changed row, one new row and one unchanged row give exactly one closed row,
   two new current rows and no third version of the unchanged row; a third
   identical load writes nothing; the number of table commits per load is
   one; reading the current rows of the load's keys from the table works
   through the catalog the sink uses, in chunks, without reading the whole
   table for a small load. It asserts what the table holds. The
   planner writes `docs/plans/SRC-10-RESULT.md` from it. If merging cannot
   keep the partition or silently keeps duplicates, stop and report.
2. **Store and validation.** `LoadMode` gains `ReplaceDedup`,
   `IncrementalDedup` and `History` (needs a key; with a cursor on a SQL
   source it reads only new rows, otherwise the whole source); `SourceObject` gains `primary_key: Option<Vec<String>>`
   (bounded count and length, no control characters);
   `validate_load_modes`: a dedup mode needs a non-empty key;
   `incremental_dedup` needs the `sql` adapter and a cursor, as
   `incremental` does. No migration: source objects are JSON. TypeScript
   contract mirrors it.
3. **Loader.** `sink.py`: `LoadPlan` carries the key; the two new modes map
   to a merge with strategy `upsert` and the key as primary key; rows are
   deduplicated on the key before loading, highest cursor value first, last
   row read when there is no cursor (D4). `replace_dedup` replaces the table
   with the deduplicated rows. `incremental_dedup` filters by the cursor as
   `incremental` does, then merges. The cursor bookkeeping (`_forget_cursors`,
   the first-run rule) is extended, not copied.
3a. **Loader: history (D1, D9).** In its own module beside `sink.py`, with
   a narrative docstring that states the three properties of task 1 and the
   limits (valid-from is the load's time; a deleted source row stays
   current). The four reserved column names are refused as source column
   names for a history table. Switching a table to or from `history` when it
   already holds rows is refused with a fixed message that names "refresh,
   removing old rows" as the way; it is never converted silently.
4. **Schema changes meet the key (D6).** In `schema_diff.rs`: for a table in
   a dedup mode, the removal of a key column is breaking even under a policy
   that would apply it. The observation route receives the table's mode and
   key from the stored spec, not from the orchestrator.
5. **One table, with an action.** `POST /api/connectors/{id}/ingest/run`
   accepts an optional body `{object, action}`; the 409 for a run already
   going and for a paused connector stay; the tenant gate stays. The run
   config carries both. `run_ingest` fans out over that one table.
   `refresh_keep`: forget the table's cursor, load by its mode.
   `refresh_remove`: empty, forget the cursor, load. `clear`: empty, forget
   the cursor, load nothing, record an `ingest_run` row with status
   `cleared`. An audit event for each, naming the table and the action.
6. **Console: modes and key.** The table list in
   `connector-ingest-panel.tsx`: six modes where the source allows them,
   four otherwise with one line saying why (D2); for `history`, one line
   saying what the three extra columns are and that valid-from is the time
   of the load; a key field per table,
   filled from discovery where the source declares one (D3); the problems the
   API would refuse are shown before saving.
7. **Console: refresh and clear.** Per table, a menu with the three actions,
   each behind a confirmation that says what happens to the rows (D5); the
   run view names the table and the action.
8. **Gate.** On the MySQL fixture, a table in `incremental_dedup`: load;
   update a row and add one; load; assert through the query engine one row
   per key with the new values and the right count (D8, acceptance row 11).
   Then refresh a second table of the same connector and assert the first
   did not change (acceptance row 3). A third table in `history`: load;
   update a row; load; load again unchanged; assert one closed row, one
   current row for that key and no extra rows (acceptance rows 2 and 12). Where this step lives is in section 5.
9. **Docs (planner).** `docs/plans/SRC-10-RESULT.md`; the spec's "Today" for
   sync modes and its history row, by name; `CHANGELOG.md`.

## 4. Assistant

`routes/ai/tools/ingest.rs` `set_ingest_spec` passes source objects through.
Its schema lists the load modes: regenerate
`tests/fixtures/tool_schemas.json` the way that file is normally written, and
the AI team reviews the diff.

## 5. Building beside `SRC-9`

- `SRC-9` rewrites how the gates are organised (`ops/g6`, `ops/gates`,
  `.github/workflows/ci.yml`). `SRC-10` touches the gate script in one
  place, task 8. Task 8 is done last, after `SRC-9`'s registry task has
  merged; until then tasks 1 to 7 need no gate change.
- Neither adds a migration.
- The build machine runs one cargo command at a time, and both worktrees use
  one cargo target directory in turn (`touch` the changed sources first; a
  clippy that finishes in seconds on changed code is not a result). Free disk
  is checked before every cargo command.
- Each has its own draft pull request; neither waits for the other except at
  task 8.

## 6. Out of scope

- Deletes at the source; CDC connectors. History by the time a change
  happened at the source (a sequence column), which Databricks also offers.
- Column selection by the user (`SRC-11`).
- Any change to `replace`, `append` or `incremental` for tables that keep
  those modes.

## Handoff

### Task 1 only (developer, stopped on a plan stop condition)

Commit `4532efc` adds `dagster/dispar_orchestrate/test_load_modes_measurement.py`
(27 tests). Tasks 2, 3 and 3a were NOT started: the history load is more than
one commit above 1000 rows and can leave a closed row without its successor
(the plan's own stop condition). Observations are in the test names and
docstrings; dlt 1.30.0, pyiceberg 0.12.0, local ephemeral catalog (not
Lakekeeper), no query engine.

Key facts: `dlt/common/libs/pyiceberg.py::merge_iceberg_table` calls
`table.upsert` per 1000 rows, each its own commit. Up to 1000 rows per load
(closed rows plus new versions) a history load is one commit and a failure
between "read" and "write" or inside the upsert commits nothing. At 1500
changed keys (3000 rows) it is three commits, and failing the second leaves
1000 keys with a closed row and no current row. One pyiceberg
`Table.upsert` on the whole batch is one commit at 3000 rows and rolls back
whole, but bypasses dlt's load step (prototype in the test).
Merge itself: day partition and format-version 2 kept; one commit; updated
row gets the new `_ingested_at`, untouched row keeps its own; a repeated key
in a batch, a null key and a table that holds duplicates all fail the load
loudly, and the failed package stays pending so later loads of that table
fail too.

Commands: `(cd dagster && ~/.cache/rantai-dagster-venv/bin/python -m pytest
dispar_orchestrate -q)`: 739 passed before, 766 passed after (27 new);
`python3 ops/lint/check_intra_package_imports.py` and
`check_bare_iceberg_count.py`: OK. No Rust, TypeScript or compose touched.
Not verified: a REST catalog (Lakekeeper), ClickHouse and Trino reading a
merged table, large-table cost.

*(developer)*

### Second pass: dedup modes (developer)

Scope: `replace_dedup` and `incremental_dedup` only; no `history` value anywhere.
Commits on `feat/src-10-load-modes` after `9927039`:

| Commit | Task |
| --- | --- |
| `01b4287` | A. Measurements for decisions 3, 4, 5 (`test_load_modes_dedup_measurement.py`, 14 tests) |
| `b671915` | B. Store, validation, TypeScript contract (Rust + TS) |
| `a421bff` | C. Loader: `adapters/dedup.py`, `sink.py`, `ingest_factory._load_plan` |
| `19a77b1` | D. Abort pending packages in `load_via_sink` |

**No stop condition was hit.** Two findings change what the planner should expect (1 and 2 below).

Observations (dlt 1.30.0, pyiceberg 0.12.0, local catalog; same limits as the first pass):

1. **Decision 3, corrects first-pass case (g).** The first pass saw pyiceberg refuse a merge into a table that
   holds a key twice ("Target table has duplicate rows"). That was true only because it loaded both copies in ONE batch
   (one data file). Copies in DIFFERENT data files, which is what two `append` runs leave, are NOT refused: the merge
   succeeds, updates both rows and the duplicate stays (`test_3_a_merge_into_a_table_that_holds_a_key_in_two_data_files_...`).
   A merge is never a repair. The first-run rule is therefore what makes the table sound: run 1 of `incremental_dedup`
   (no cursor state) replaces with the winners, run 2 merges (`test_switching_an_append_table_with_duplicates_to_incremental_dedup_...`:
   10 rows from two appends of a 5-row source -> 3 rows, then a merge of one changed row -> 3 rows). The rule covers
   the `append` case as asked.
   **Not covered (reported, not fixed):** switching a table from `incremental` to `incremental_dedup`. It is the same
   cursor, so run 1 finds cursor state and merges instead of replacing, into a table that holds an old and a new version of
   a key, updates both and leaves the duplicate, silently (`test_3_a_table_with_two_versions_of_a_key_from_incremental_...`).
   A refresh that forgets the cursor (plan task 5) is the way out; the planner may want the console to say so, or a
   marker that distinguishes "cursor of incremental" from "cursor of incremental_dedup".
   Also measured: the cursor a replacing first run leaves is the one the next merge continues from (nothing new -> no commit).
   The run that stores the cursor-boundary hash and the runs that compare it must pass the same `primary_key`, or the last
   row is re-read and re-merged every run, so the key goes on the first (replace) run too.
2. **Decision 4.** `Pipeline.drop_pending_packages` is deprecated in 1.30.0 and calls `abort_packages`, which also restores
   the pipeline state to the point before the package. After a failed load in each family (merge, replace, append, incremental,
   normalize failure from a null key) aborting lets the next clean load through, loses and doubles no row, and the
   incremental cursor does not advance past rows never loaded (rows 4 and 5 are loaded once by the next run). Without the abort,
   dlt logs "will now load the pending load packages. The data you passed ... will not be extracted": the next run reports
   success for the PREVIOUS run's read, `last_normalize_info` is `None` and `sink._extract_rows` raised `AttributeError`
   (fixed). Applied to every mode; no mode lost or doubled a row, so none was excluded.
   **Gap, not changed:** a package left by a PROCESS killed mid-load is not seen until the next run, which would then finish it
   instead of reading (for a Kafka micro-batch that means its offsets would be committed for rows not loaded). Aborting at
   the START of a run would close it; it is outside the decision's wording ("when a load raises or ends with failed jobs").
3. **Decision 5.** A merge above 1000 rows is several commits: a failure injected in the third upsert of a 2,500-row merge
   leaves 2 committed slices, one row per key (2000 new + 500 old), never a key twice, and the next clean run brings the table to
   the source's state (through the loader: `result.rows == 2500` because the abort restored the cursor). Stated in the
   `sink.py` docstring. A partial merge cannot leave two rows for one key because the batch has one row per key and each slice
   is upserted alone.
4. **Decision 2 / null key.** `dlt` wraps an exception from a pipe step twice (`ResourceExtractionError` inside `PipelineStepFailed`);
   `sink._refusal_in` walks the chain and re-raises the `DedupRefused`, a `ValueError` (so the op boundary's existing
   `ValueError` arm gives `Failure(allow_retries=False)`); `ingest_factory._run_one_object` records it as `rejected`
   (added to its tuple). Message: `table 'n': 2 row(s) have no value for key column 'id'; a deduplicating load needs every row to
   carry its key` (asserted in full; a test puts secret text in the rows and checks it is absent).
5. An empty plain-rows source (`[]`) empties the table under `replace`; an empty `dlt` resource leaves it untouched. The dedup step
   does the same as `replace` in both cases (`test_replace_dedup_of_an_empty_source_does_what_replace_does_with_it`).

What the dedup step costs: memory proportional to DISTINCT keys (a dict key tuple -> best row so far), not rows read
(`test_memory_is_the_number_of_distinct_keys_not_the_number_of_rows`: 10,000 rows, 10 keys, 10 rows held). Nothing reaches
the sink until the read ends (a stage the read waits at), so a large `replace_dedup` writes nothing while it reads. The bound is read
from `INGEST_DEDUP_MAX_ROWS` by the frozen dataclass `DedupLimits.from_env()` in `adapters/dedup.py` (default 2,000,000
distinct keys; an unreadable value fails the load, closed); above it the load fails with `TooManyKeys` naming the setting. It
counts distinct keys although it is named for rows; the docstring says so. Not measured: wall time and memory at 2,000,000 keys.

Order of the steps in a `sql_database` pipe, asserted: gen, incremental cursor step, `DedupStep`, stamp
(`test_the_pipe_runs_the_cursor_filter_then_the_dedup_then_the_stamp`); the stamp is called once per winner, not per row
(`test_the_stamp_is_applied_to_the_winners_only`); SRC-8's `table_adapter_callback` (`keep_only`) works with both modes, and a
key column removed by it fails as a missing key. `dlt` has no end-of-stream hook for a pipe step, so the step wraps the data
generator to append an empty-list marker that the incremental step forwards untouched; a pipe with any other kind of step is refused
(a filter could swallow the marker and the winners would never be emitted), and a run that did not flush fails (`assert_flushed`).
Kafka micro-batches take no source-object load mode: `load_via_sink(batch.rows, ...)` keeps `append`. Files, REST, `MongoDB` and SFTP
get `replace_dedup` on plain rows through `dedup_rows`; `incremental_dedup` is refused for them by `_load_plan`.

Rust / TypeScript (task B): `LoadMode` serde moved from `rename_all = "lowercase"` to `"snake_case"` (the three old values serialise the
same). The incremental error messages now read "an incremental loadMode" (they cover both modes) and the unknown-mode message lists
five modes; no test elsewhere pinned the old text (grep). The assistant's `set_ingest_spec` schema does not list `loadMode` or any
mode (its source-object properties are `name`, `incrementalKey`, `target`), so **`tests/fixtures/tool_schemas.json` and
`registry.rs` are UNCHANGED**; plan section 4 ("its schema lists the load modes") does not match the code. The console panel's
`LOAD_MODE_LABEL` is an exhaustive `Record<LoadMode, string>`, so it gained labels and hints for the two modes; they are not offered
(`loadModesFor` unchanged, no key field) but `primaryKey` and `incrementalKey` are now carried through a save so the console does not
silently undo a table the assistant set up.

Commands and results (all foreground except where noted):

- `(cd dagster && ~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate -q -p no:cacheprovider)`: at `01b4287` 780 passed
  (766 + 14); at `a421bff` (C as committed, with a 2,500-row test that needs D) 844 passed, 1 failed (that test), the test then moved to
  D's file and `test_dedup.py` + `test_sink_dedup_modes.py` re-run alone: 64 passed; at `19a77b1` 854 passed (a first run of D showed 2 failures,
  the two assertions named below, fixed). 854 = 766 + 14 + 26 + 38 + 10. `b671915` touches no Python: the suite state is `01b4287`'s.
- `python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py`: OK, OK (before C and D commits).
- `cd rust && cargo fmt --check`: first run flagged two hunks of mine in `ingest_spec.rs`; `cargo fmt` changed nothing else; then OK.
  `df -h /` before cargo: 17 GB free. After `touch` of `ingest_spec.rs`: `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2
  CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: Finished in 2m 20s, no warning
  (it rechecked `lakehouse-store`, `-auth`, `-api`). Run detached (`setsid nohup`) and polled, because a foreground call is capped at 10 minutes.
- `bun install --frozen-lockfile`, then `bun run typecheck && bun run lint && bun run test`: typecheck and lint clean; test 1067 pass, 1 skip,
  1 fail on two consecutive runs (`ConnectorDetailPage > marks a table that waits for a schema decision ...`, "Schema changes1" vs
  "Schema changes"), then 1068 pass, 1 skip, 0 fail on a third; that file alone passes 28/28 twice; `src/features/connectors` alone 205/0.
  The same suite on the base (stash) passed 1068/0. I could not reproduce the failure in isolation; treat it as a timing flake
  and *not* proven unrelated.

Assertions changed (one place): in `test_load_modes_dedup_measurement.py`, `test_4_without_the_abort_the_next_run_only_finishes_...` and
`test_4_without_the_abort_the_next_incremental_run_finishes_...` (written in `01b4287`, not first-pass tests) ran a failing load through
`load_via_sink`; after D that call aborts, so they switch `sink._abort_pending` off to keep measuring dlt without it. No first-pass test was
changed.

Not verified: `cargo test` for anything (the new `ingest_spec.rs` unit tests have been type-checked by clippy `--all-targets` but not run; CI);
sqlx/API integration tests; a Lakekeeper REST catalog; ClickHouse or Trino reading a merged table (gate, plan task 8); behaviour and
memory at 2,000,000 distinct keys; a `MongoDB`/SFTP/files run through `replace_dedup` end to end (only plain rows and a dlt resource
are tested, not those adapters); the compose/CI path. Nothing was pushed and the PR was not touched.

Plan/code mismatches: (a) section 4 says the assistant's tool schema lists the load modes; it does not. (b) The first pass's case (g) conclusion
("a merge into a table with duplicates fails") is true only for duplicates in one data file. (c) The plan's "first-run rule" does not cover
`incremental` -> `incremental_dedup`.

*(developer)*

### Third pass: review findings of the second pass (developer)

Scope: the planner's three findings and the CI flake. `history` is still waiting for the product owner: no `history` value, no task 3a.
Commits on `feat/src-10-load-modes` after `d9df91e`:

| Commit | Item |
| --- | --- |
| `01c710f` | R1 the failing Rust test (CI, PR #115) |
| `a83ce0e` | R4 the frontend test that failed twice and then passed |
| `fb6ab82` | R2 `incremental` -> `incremental_dedup` leaves duplicates |
| `90a6976` | R3 a run killed mid-load |

**R1.** `ingest_run.rs:432` expected `"replace, append or incremental"`; the list grew, so the expectation is CORRECTED to `"replace, append, incremental, replace_dedup or incremental_dedup"` (the whole list, in order, `contains` as before: not loosened). `cargo test` stopped at that binary (`grep -a "Running "` of the CI log: it ran 22 binaries and stopped inside `tests/ingest_run.rs`; the remaining `lakehouse-api` tests and the other crates never ran). Searched for pinned mode lists or messages in `rust/**`, `src/**/*.test.*`, `dagster/**`, `ops/**` (g6, g9): the other places that name modes are the uploads' own `LoadMode` (`replace|append`, a different enum in `uploads.rs`, `upload_routes.rs`, `tests/uploads.rs`), `"loadMode": "replace"` fixtures (`store/tests/connectors.rs`, `schema_change.rs`, `ops/g6`) and the `ingest_spec.rs` unit tests, which match error VARIANTS, not text. No other test pins the old list or the old "an incremental loadMode" wording. `cargo fmt --check` clean; clippy (workspace, all targets, all features, locked, `-D warnings`) Finished in 2m 21s, no warning.

**R2.** The marker is `_dedup_key`, a list of the key columns, in the RESOURCE state of the pipeline state (`pipeline.state["sources"][schema]["resources"][resource]`), the dict that also holds the incremental cursor `_incremental_cursors` reads. It is written by `dlt.current.resource_state()` while the rows are extracted (`DedupStep` at its flush, `dedup_rows` for plain rows; only when rows were emitted), so it is part of the extract package: stored with the data in the bucket and restored by `abort_packages` with the rest of the state. `incremental_dedup` is a first run (replace) when the marker is not exactly the plan's key or no cursor exists; if a cursor exists but the marker does not match, `_forget_cursors` runs first (so the run reads everything). `replace`, `append`, `incremental` clear it from the `_ingested_at` stamp (`_stamp_for_load(..., clear_marker=True)`), which adds no pipe step and no read of the state for `replace`/`append`. Not cleared: a plain-rows `append` (the Kafka micro-batch; no dedup mode shares a table with it). `test_sink_dedup_marker.py`, 9 tests through `load_via_sink`; measured: a failed first run leaves no marker and is a first run next time; a pre-existing marker `["id"]` is back to `["id"]` after a failed run that had written `["id","v"]`; the marker is read back by a new `DLT_DATA_DIR` and the next run merges (1 commit, untouched rows keep `_ingested_at`). With the comparison bypassed, 4 of the 9 fail. The `sink.py` docstring states the rule. Known limit: a stale marker in ANOTHER source schema of the same pipeline name is also read (the walk is across all schemas, as for cursors).

**R3.** Measured first (`test_load_modes_dedup_measurement.py`, case 6, abort switched off): with a state already in the bucket, the run after a kill (simulated by a `BaseException` out of `Pipeline.normalize` or `.load`) finishes the dead package and does not extract its own read; Kafka-shaped append: table holds the dead batch's rows (0..3), not the live ones (4, 5), `rows` is 3 (killed before normalize) or `None` (before load), never 2, `has_failed_jobs` false; an incremental run reports `rows == 0` while it loaded the dead package's row 4 and its own row 5 waits one run. With NO state in the bucket yet `dlt` wipes the working folder at its restore, package included (measured; no abort needed). Now `load_via_sink` calls `_abort_pending(pipeline, strict=True)` right after creating the pipeline (strict: a failed abort raises). `test_sink_killed_run.py`: 13 tests, replace, Kafka-shaped append, incremental (cursor not past rows never loaded; a following run reads 0), replace_dedup, incremental_dedup (+ marker), a killed first dedup run, a failed abort, no abort call when nothing is pending; with the abort removed the first fails.
Kafka: `run_kafka_stream_batch` commits (`consumer.commit`, then `record_ingest_offset`) only after `load_via_sink` returns, and a returned call now always means THIS batch's rows were extracted and loaded (the dead package is gone). Offsets can no longer run ahead of loaded rows through a killed run. Two paths remain, NOT changed here: (a) a process killed after the load and before `consumer.commit` re-delivers the batch, so its rows are loaded twice (at-least-once; duplicates, not loss); (b) `ingest_factory.py` does not look at `outcome.has_failed_jobs` before committing, so a sink that returned with failed jobs would still commit offsets; dlt raises on failed jobs by default, so it was not reached in any test, and it is a one-line check for the planner to decide.

**R4.** Cause: the test awaited `findAllByText("Waiting for a decision")`, but that is also the title of the Schema changes card, which is on the page before the list has loaded, so it waited for nothing; the tab's count is set by an effect of the schema panel after the list arrives. Reproduced deterministically by delaying `GET .../schema-changes` 150 ms: the old assertion fails ("Schema changes" received), the new one passes. Fix (test only, the component is right): wait for the mark itself (`findAllByTitle` of the pill's own title), assert exactly one, on `public.orders` and not on `public.items`, then `waitFor` the tab's count and assert `"Schema changes1"`; nothing it checked is weakened. Loops: the file 20 times, 20 pass 0 fail; `src/features/connectors` 5 times, 5 pass 0 fail.

Commands and results (foreground unless noted):

- `cd rust && cargo fmt --check`: OK. `df -h /`: 17 GB free. After `touch`: `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` (detached, polled): Finished in 2m 21s, no warning.
- Dagster, whole suite, detached and polled: after R2 (marker, without R3) 867 passed; after R3 a first run 875 passed, 5 failed (three `FakePipeline` stand-ins without `has_pending_data`, two lambdas without the new keyword; fixed); at `90a6976` **880 passed** (854 + 9 + 4 + 13). 880 passed in 526.69 s.
- `python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py`: OK, OK.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (none in files touched). `bun run test`: 1068 pass, 1 skip, 0 fail.

Assertions and helpers changed: `ingest_run.rs` (R1, above); `connector-detail-page.test.tsx` (R4, above); the three `_FakePipeline` classes (`test_sink.py`, `test_dlt_pipeline.py`, `test_file_ingest.py`) gain `has_pending_data = False`; in the measurement file two `monkeypatch.setattr(sink_module, "_abort_pending", lambda _pipeline: None)` take `*_a, **_k`, and the docstring of `test_3_a_table_with_two_versions...` now says the gap is closed by the marker. No assertion was removed or loosened.

Not verified: `cargo test` (not run on this machine; CI), so `ingest_run.rs` passing with the corrected text is verified by reading the message in `ingest_spec.rs`, not by running it; a Lakekeeper REST catalog, ClickHouse, Trino (gate); a real SIGKILL (a `BaseException` out of `Pipeline.normalize`/`.load` stands for it); a recreated container with a real second process (a new `DLT_DATA_DIR` stands for it); wall time and memory at 2,000,000 keys; the compose/CI path. Nothing was pushed and the PR was not touched.

*(developer)*

### Fourth pass: schema changes and the key, refresh and clear, the console (developer)

Scope: plan tasks 4 to 7. `history` (task 3a) is still waiting for the product owner: no `history` value, no option for it.
Commits on `feat/src-10-load-modes` after `d03ee44`:

| Commit | Task |
| --- | --- |
| `13e713a` | 4 schema changes meet the key (decision 6) |
| `33a4715` | 5 one table, with an action (decision 5), API and orchestrator |
| `81d2fad` | 6 console: modes and key (decisions 2, 3, 4), discovery returns the key |
| `3fd067d` | 7 console: refresh and clear (decision 5) |

**Task 4.** The table's mode and key are read from the STORED spec (`ingest_spec::dedup_keys`) by the observation route and again by `list_pending`, by `record` and by `approve_object`, so a row becomes approvable the moment the key or the mode stops naming its column. `schema_diff::decide_for_key` / `evaluate_for_key` (`decide` and `evaluate` are the same with no key). The removal of a key column waits under every policy, `canApprove` is false for the table's waiting changes, approval is a 409 with a fixed text, and the connector is NOT paused by it (SRC-8 review BLOCKER 3a: a pause nobody can lift by approving sticks). A changed source primary key stays approvable: the declared key is still loadable; a removed key column is not. Wire change: the store's `ApproveOutcome` has a fourth variant, `KeyStillNamed`.

**Task 5.** Measured first (`test_sink_empty_table.py`): `load_via_sink([], ..., replace)` empties a table that plain rows created (earlier measurement) but leaves all rows of a table a `sql_database` source created. `sink.clear_table` therefore runs `pipeline.run([], table_format="iceberg", write_disposition="replace")` straight on the table's pipeline: the table is emptied and keeps its columns, the day partition on `_ingested_at` and format-version 2; an empty `dlt` resource leaves the table untouched; a never-loaded table stays absent; idempotent. `LoadPlan.refresh` (`keep` | `remove`) forgets the table's cursor AND dedup marker (`_forget_table_state`); `remove` empties first, inside `load_via_sink`, after the adapter's schema check. For a deduplicating mode "keeping" already replaces with the deduplicated source (the first-run rule), so keep and remove end in the same rows; only `append` differs ("keeping" = a second full copy, like Airbyte's refresh-retain). `ingest_run.status` is a free `String` in `EXPECTED_SCHEMAS`: `cleared` is stored with rows 0. A `clear` resolves no secret, dials no source and makes no schema observation, so `connector_source_schema` is untouched.

**Task 6.** Discovery now returns `primaryKey` (see report); three const queries, one bind each, MySQL names cast to `CHAR`.

**Task 7.** The Runs card state moved into `useIngestRunState`, owned by the editor.

Commands and results: `cd rust && cargo fmt --check`: OK. `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` (detached, polled, after `touch`): Finished, no warning, after each of the tasks 4, 5 and 6 (2m12s, 2m25s, 1m45s). Dagster whole suite, detached: 911 passed in 576 s (880 + 31 new). `check_intra_package_imports.py` and `check_bare_iceberg_count.py`: OK. `bun run typecheck`, `bun run lint` (0 errors, 6 warnings, none in files touched), `bun run test`: 1096 pass, 1 skip, 0 fail.

Assertions changed (all in `connector-ingest-panel.test.tsx`): the two `loadModeProblem` lines that expected `null` for `incremental_dedup` with a cursor and for `replace_dedup` now give a key, because a deduplicating mode without a key no longer saves (decision 3); the two option-list assertions grew by the new modes. Test literals of `IngestJobRun` get `object: null, action: null`. Nothing else changed.

Not verified: every Rust test (`cargo test`, the new `#[sqlx::test]`s in `schema_changes.rs`, `connector_discover.rs`, `load_alerts.rs`, `tests/ingest_run.rs`; CI); the MySQL and SQL Server primary-key queries on a real server (no fixture; SRC-9's gate); a Lakekeeper REST catalog for `clear_table`; ClickHouse and Trino reading a cleared or refreshed table; the console in a browser.

Plan/code mismatches: (a) plan task 5 says "empty ... through the sink": the `load_via_sink([])` form does not empty a table a SQL source created (above), so `clear_table` bypasses `iceberg_adapter`. (b) `routes/load_alerts.rs` skips BOTH a successful and a failed `clear` run (decision: a clear is neither a success nor a failure of a load); a failed clear is therefore visible in the run list and the `ingest_run` row, not as an alert; the planner may want a failed clear to alert. (c) A refresh that fails after `remove` emptied the table leaves it empty until the next successful run (stated in the confirmation text).

*(developer)*

## Review

*(planner)*
