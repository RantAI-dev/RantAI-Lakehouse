# DATA-31: a typed workbook load keeps its bad-values report where the API reads it

Backlog `DATA-31` (Task, Data, P2). Reassigned to the planner lane on 2026-11-01. Base: `main` at
`a33e3d81`. Planner: Claude Opus. Developer: another agent (`AGENTS.md`). If the code disagrees, stop
and report it.

## What is wrong (confirmed)

- **The job writes the report beside the wrong object.** It writes
  `{bucket}/{keys[0]}.bad-values.json` (`dagster/dispar_orchestrate/file_ingest.py:1032-1055`,
  `_keep_report` `:1235-1246`, called at `:1287`, with `key = keys[0]` at `:1261-1262`).
- **For a workbook, `keys[0]` is the converted sheet,** `<storage_key>.converted.<n>.csv`
  (`routes/uploads.rs:3099`, `converted_text_key` `:604`).
- **The API reads it beside the original,** at `<storage_key>.bad-values.json`
  (`routes/uploads/columns.rs:499`, from `uploads.rs:1577`). So the person never sees a typed
  workbook load's bad values.
- **Also found:** `delete` (`uploads.rs`, about `:3339-3415`) never removes
  `<storage_key>.bad-values.json`, for any format.

## Decisions

- **D1. Change the job, not the API.** The naming rule (plan AD6,
  `2026-10-16-data-phase-3-uploads.md:91`) puts every object of an upload beside the **original**.
  - In `_load`, derive `report_key` from `keys[0]` by stripping a trailing
    `.converted.<digits>.csv`, or the legacy `.converted.csv`, and pass it to `_keep_report`.
  - One helper, with a docstring citing AD6 and `DATA-31`.
  - The run config does not change.
- **D2.** `delete` also removes `<storage_key>.bad-values.json`. A missing object is fine, as for
  the other objects `delete` removes.

## Task (one commit)

- **T1.** Make D1 and D2. Cite `DATA-31`.
- Tests:
  - in `test_typed_load.py`, a converted-key load stores its report under the original's key, and
    a clean converted-key load removes that report;
  - in `tests/upload_routes.rs`, a settled typed **workbook** load carries the report (extend
    `a_settled_typed_load_carries_the_report…`, about `:6260`), and delete removes the report
    object.
- *Accept:* `(cd dagster && python -m pytest dispar_orchestrate/test_typed_load.py dispar_orchestrate/test_file_ingest.py -q)`;
  `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
  `cargo test -p lakehouse-api --test upload_routes typed`;
  `cargo test -p lakehouse-api --test upload_routes delete`.

## Handoff (developer appends below)

## Review (planner appends below)
