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

### Handoff (developer, 2026-11-01)

**Commit.** `98794f98` — "DATA-31: a typed workbook load keeps its bad-values report where the API
reads it" (5 files: the job, its tests, the API's `uploads.rs` and `columns.rs`, and
`tests/upload_routes.rs`). Branch `fix/data-31-workbook-report-key`, pushed to `origin`.

**D1 (the job).** `_report_key` in `dagster/dispar_orchestrate/file_ingest.py` derives the report's
key from the first storage key by stripping a trailing `.converted.<n>.csv` (or the legacy
`.converted.csv`); `_load` passes it to `_keep_report`. The run config does not change. Checked
against the API's own names (`converted_text_key`, `routes/uploads.rs:604`).

**D2 (delete).** `DELETE /api/uploads/{id}` deletes `<storage_key>.bad-values.json` too, for every
format (`routes/uploads.rs`, after the object's own delete). A missing object is a no-op, as for
the others (`UploadStore::delete` answers Ok for a missing key). `REPORT_SUFFIX` and the new
`report_key` moved to `routes/uploads.rs` beside the other key helpers, so one place owns the
report's key; `columns.rs` uses the same helper.

**Commands run** (the plan's accept list, in the foreground; every cargo command through
`bin/cargo-locked`):

| Command | Result |
| --- | --- |
| `(cd dagster && .venv/bin/python -m pytest dispar_orchestrate/test_typed_load.py dispar_orchestrate/test_file_ingest.py -q)` | 228 passed, 1 skipped, 10 warnings in 80.07s |
| `bin/cargo-locked fmt --check` | clean (after `bin/cargo-locked fmt`, which reflowed two of the new test lines) |
| `bin/cargo-locked clippy -p lakehouse-api --all-targets -- -D warnings` | finished, no warnings |
| `bin/cargo-locked test -p lakehouse-api --test upload_routes typed` | 2 passed, 0 failed, 172 filtered out |
| `bin/cargo-locked test -p lakehouse-api --test upload_routes delete` | 6 passed, 0 failed, 168 filtered out |
| `python3 ops/lint/check_intra_package_imports.py` | OK: all intra-package `dispar_orchestrate` imports resolve |
| `python3 ops/lint/check_bare_iceberg_count.py` | R11 guard: no bare `count()` against a Bronze Iceberg table |

The three new Python tests (`-k converted`: 3 passed) and the two touched Rust tests are inside
those counts. The testcontainers the Rust tests start were removed afterwards.

**Environment.** `dagster/.venv` did not exist and the machine's `python3` (3.14) has no `dlt` and
no `pytest`, so the venv was created there (`uv venv --python 3.12 --seed .venv`, then
`uv pip install -e . pytest`); the pytest command above is that venv's interpreter. The venv is
ignored by git (`dagster/.venv/.gitignore`).

**Not verified.**

- The full workspace run (`cargo clippy --workspace --all-targets --all-features`,
  `cargo test --workspace`) was **not** run: the machine rules for this branch say to run only the
  scoped commands the plan lists, and the reviewer runs the full block before merge.
- `bun run typecheck`, `bun run lint`, `bun run test` were **not** run: no TypeScript changed.
- `(cd dagster && python -m pytest dispar_orchestrate -q)` (the whole Dagster suite) was **not**
  run: only the two files the plan lists.
- `docker compose config` and a real `compose up` were **not** run: no compose file changed.
- The `g9-upload` gate (`ops/g9/upload_test.py`) was **not** run: it needs a deployed stack. It
  exercises a typed CSV load, whose report path is unchanged.

**Left for the planner.** The feature page `docs/core/features/upload-column-types.md` still lists
the gap (`DATA-31`) in "Limits to tell a customer" and in the "Bad values" row of the spec
targets; the `BACKLOG.md` row for `DATA-31` is still `Todo`.

## Review (planner appends below)
