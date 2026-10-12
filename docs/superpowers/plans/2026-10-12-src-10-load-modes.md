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

## Review

*(planner)*
