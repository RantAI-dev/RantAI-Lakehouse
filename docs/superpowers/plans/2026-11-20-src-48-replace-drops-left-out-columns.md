# SRC-48: a replaced table drops the columns left out of it

Backlog: `SRC-48` (Task, Data, P2), added in PR #620.
Base: `origin/main` at `b1df2c451`.
Branch: `fix/src-48-replace-drops-left-out-columns`. One PR.

## Why

`docs/core/features/connector-operations.md`, decision 10, signed
2026-10-14:

> The next load does not read the column from the source. In a table that
> is replaced on every load the column is gone after that load. In a table
> that keeps its rows the column stays, with no new values, and is marked
> inactive.

QA `TC-1706` retested this on staging on 2026-11-20 (build `dc9ce2f7`),
using alfamei POS, `public.kasir` (load mode Replace all rows):

1. `shift` was unticked.
2. The next load (05:21 UTC) wrote `shift` as NULL in every row, and the
   column stayed in `iceberg.bronze.alfamei_pos_kasir`.
3. `shift` was ticked again, and the next load filled it.

The first half of decision 10 holds: the column is not read. The before-load
gate narrows the decision in `_leave_out_columns`
(`ingest_factory.py:677`). The second half does not: a replace keeps the
table's schema. The product owner chose to fix the code to the decision
(2026-11-20).

## Decisions (made, not to be changed by the developer)

- **D1. Which tables drop the column.** Only a source object whose load
  mode replaces the table on every load: `replace` and `replace_dedup`
  (`LOAD_MODES`, `sink.py:177`).
  - `incremental`, `incremental_dedup` and `history` keep their rows, so
    they keep the column. This holds even on a first incremental run,
    which happens to write as a replace (`sink.py:1234-1246`).
  - Only the SQL batch path has excluded columns (`ingest_factory.py:131-134`),
    so only that path passes any.
- **D2. Which columns are dropped.**
  - The object's `excludedColumns`, each mapped to its Bronze name with the
    pipeline schema's own naming convention, keeping only those the
    Iceberg table actually has.
  - Never a `SYSTEM_COLUMNS` entry (`sink.py:829`), never a key column,
    never the cursor column. The picker already locks the key and cursor
    (decision 11); the sink refuses them again (fail closed, principle 3):
    a key or cursor name in the list is skipped and logged by name to the
    log only, never to a response.
  - A column that disappeared at the source is not touched here. That is
    `SRC-8`'s schema-change path, which keeps it inactive.
- **D3. How the column is dropped.**
  - Before the `dlt` run of that replace, the sink removes the columns in
    two places:
    1. the pipeline's stored `dlt` schema for that table, so `dlt` does not
       write the column as nulls again;
    2. the Iceberg table, with one `pyiceberg` schema commit
       (`update_schema().delete_column`), recorded with the run's author
       like every other sink commit (`history.snapshot_properties`,
       `DATA-52`).
  - Then the replace runs as today.
  - The table is never dropped or purged (the `DATA-18` rule).
  - This is not one commit with the rows. A load that fails after the
    schema commit leaves the old rows without that column until the next
    run. That is accepted: the person asked for the column to be left out,
    and the next successful run replaces the rows. Say so in the
    function's doc comment.
- **D4. Ticking the column again.** The next replace reads the column and
  `dlt` adds it back, so it is filled from that load (decision 10's
  "Limits" line). No extra code is expected. The test in task 3 proves it.
- **D5. Stop condition.** If the measurement in task 1 shows that `dlt`
  writes the column back even with both removals done, stop and report the
  measurement in the Handoff. Do not try another mechanism, such as a
  dlt-bypassing write. That would be a planner decision.
- **D6. Out of scope:**
  - the `excluded` inactive marks in the store
    (`lakehouse-store/src/schema_change.rs:1200`): they stay as they are;
  - other adapters;
  - any API, Rust or console change;
  - SRC-49.

## Anchors (verified at `b1df2c451`)

- `dagster/dispar_orchestrate/adapters/sink.py`
  - `:177` `LOAD_MODES`
  - `:184` `INCREMENTAL_MODES`
  - `:829` `SYSTEM_COLUMNS`
  - `:847` `_retype_in` (an example of `update_schema` inside a transaction)
  - `:917` `_replace_changing_types`
  - `:1085` `load_via_sink(source, bronze_table_name, config, plan, column_types)`
  - `:1210-1246` how the write disposition is chosen
  - `:1248` where a replace opens the table (`history.open_table(pipeline, …, sync=False)`)
- `dagster/dispar_orchestrate/ingest_factory.py`
  - `:131-134` (module doc: excluded columns, SQL batch path only)
  - `:677` `_leave_out_columns`
  - `:711` `_make_gate` (`excludedColumns` read at `:717`)
  - `:940` the SQL path's `load_via_sink` call
- `dagster/dispar_orchestrate/test_typed_load.py:440-480`: the local
  filesystem-Iceberg harness (`dlt`'s own catalog), which the new tests
  reuse. No network.

## Tasks

One commit each, in order. Python only. Per commit, run only the touched
test file:
`cd dagster && python -m pytest dispar_orchestrate/<file> -q`.
The full verification runs in GitHub CI (owner's instruction 2026-11-20).
Do not run the whole suite or any build locally.

1. **Measure, then reproduce.**
   - Add a test that loads a replace with columns `a, b, c` into the local
     harness, then a second replace whose source carries only `a, b`
     (as the gate leaves `c` out).
   - Assert today's behaviour first, to record the bug: `c` is still in the
     Iceberg schema, with nulls. Note in the Handoff which of the two
     stores (dlt schema or Iceberg) kept it.
   - This test is replaced in task 3; it exists to prove the mechanism of
     D3 before the fix.
2. **The sink drops the columns (D1–D3).**
   - `load_via_sink` gains `drop_columns: Collection[str] = ()`, which are
     Bronze names.
   - When `plan.mode` is `replace` or `replace_dedup` and the table exists,
     it removes the intersection of `drop_columns` with the table's columns
     (minus system, key and cursor names) from the stored `dlt` schema and
     from the Iceberg table, before the run.
   - An empty intersection does nothing and makes no commit.
   - Docstring: what is dropped, when, and the accepted failure window
     of D3.
   - `# Errors`: whatever the catalog raises. A failure before the load
     fails the load (fail closed). The table keeps its column then.
3. **Prove it (replaces the task 1 test).** Tests in the same harness,
   with full-sentence names:
   - a replace with `c` left out leaves no `c` in the Iceberg schema, and
     the rows have the right values in `a` and `b`;
   - `replace_dedup` does the same;
   - an `incremental` table keeps `c` (with nulls in new rows);
   - a key column or a system column in `drop_columns` is not dropped;
   - ticking `c` again (a third replace carrying `a, b, c`) brings `c`
     back, filled;
   - a name the table does not have changes nothing and makes no schema
     commit.
4. **The SQL path passes the columns.**
   - In `ingest_factory.py`'s SQL path (`:940`), pass the object's
     `excludedColumns` mapped to Bronze names. Map them with the same
     naming the sink uses; reuse a helper if one exists (rule 4: grep
     first).
   - A unit test with a stub `load_via_sink` asserts the names passed for
     an object with `excludedColumns` and `loadMode: replace`, and that an
     object without exclusions passes none.
5. **Feature page and handoff.**
   - In `docs/core/features/connector-operations.md`, add one line under
     "Limits to tell a customer": *"A column left out of a table that is
     replaced on every load is removed from that table at the next load;
     ticked again, it comes back from the load after."*
   - Append the "Handoff" entry below.

## Verification

Locally, only the touched test files (counts in the Handoff). CI runs:

```bash
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && python -m pytest dispar_orchestrate -q)
```

Write those as *not verified locally, runs in CI*. Before each commit run
`git status`. Never stage `.env.staging` or `AGENTS.md`, which carry local
changes on purpose.

## Handoff

(developer appends here)

## Review

(planner appends here)
