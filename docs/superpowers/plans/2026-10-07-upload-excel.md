# Upload an Excel workbook — Implementation Plan

**Status:** asked for by the product owner on 2026-10-07 ("allow .xls and
.xlsx too"), not started. Written by the planner (Claude Opus) for a
developer agent. The planner writes no product code.

**Base:** branch `feat/upload-excel`, created from `fix/upload-large-files`
(pull request #73), because that fix carries the console checks this plan
changes. Feature page: `docs/core/features/upload-file.md` (DATA-9). Decision
record: `docs/adr/0014-file-upload.md`, amended below.

**Goal, in the user's words:** the upload accepts `.xls` and `.xlsx` as well
as CSV and TSV.

---

## 1. What the user can do when this is done

1. Choose an `.xls` or `.xlsx` file on Sources → Upload file, up to 50 MB.
2. On the Check step, pick which sheet to load (the first visible one is
   chosen to begin with) and which row is the header, and see the first
   rows as they will be loaded.
3. Name the table and load it, as with a CSV.

Not included: `.xlsm`, `.xlsb`, `.ods` (still refused, with the reason);
loading several sheets in one upload; typed columns (every column stays
text, as for CSV); formulas (the stored result is loaded, not the formula);
password-protected workbooks (refused, with the reason).

## 2. Decisions (the planner's defaults; the product owner may change any)

1. **A workbook is turned into delimited text once, in the API, and then
   takes the existing path.** The file is stored as it arrived. Preview
   converts the chosen sheet in memory and runs the existing preview on the
   result. Ingest converts the chosen sheet, stores the result as a CSV
   object beside the original, and launches the existing load job on that
   object with a fixed dialect (UTF-8, comma). **The load job does not
   change**, so there is still one reader of the loaded bytes
   (ADR 0014's invariant).
2. **How a cell becomes text** (every column is text):
   - empty: empty text;
   - text: as stored;
   - number: a whole number without a decimal point, any other number in
     its shortest form that reads back to the same value, never the cell's
     display format (`1234.5`, not `1.234,50`);
   - date and date-time: ISO 8601, `YYYY-MM-DD` when the time is midnight,
     otherwise `YYYY-MM-DD HH:MM:SS`; a time-only cell `HH:MM:SS`;
   - boolean: `true` / `false`;
   - an error cell (`#DIV/0!`): the error's own text;
   - a formula: its stored result, by the rules above;
   - a merged range: its value in the top-left cell, the rest empty, as the
     file stores it.
3. **Sheets:** every sheet is listed by name in workbook order; the first
   visible sheet is the default; a sheet with no cells can be chosen and
   then reads as an empty file does (the existing refusal).
4. **Limits:** the file limit stays 50 MB. A sheet of more than 5,000,000
   cells (rows × columns of its used range) is refused with a sentence
   saying so: a workbook is compressed and can hold far more than a CSV of
   the same size, and the API converts it in memory. The number is a cap,
   not a measurement; the feature page says so.
5. **No migration.** Whether an upload is a workbook is read from its
   stored name. The chosen sheet is recorded in the audit event and in the
   load's run configuration, not on the upload row; the uploaded-files
   list does not show it. (The dev database's migration record is still
   to be corrected after the last merge; this plan does not add to that.)
6. **Refusals are fixed sentences**, as now: a workbook that cannot be
   opened, an encrypted one, an unknown sheet name, a sheet over the cap.
   A zip that is not a workbook keeps today's refusal.
7. **One new dependency** for reading workbooks (the `calamine` crate is
   the candidate: it reads `.xls` and `.xlsx`). The developer checks its
   licence against `rust/deny.toml` and that `cargo deny` and `cargo
   audit` stay clean; if they do not, stop and report.

## 3. ADR 0014, amendment

ADR 0014 says only delimited text is accepted. Amended on 2026-10-07: a
workbook is accepted by converting one sheet to delimited text in the API
(decision 1). The two-readers-one-dialect invariant is unchanged, because
the load job still reads delimited text and nothing else; the conversion
is a third component whose output is pinned by fixtures.

## 4. Anchors (at `e352885`; re-find by symbol)

- `rust/crates/lakehouse-api/src/upload_parse.rs`: `Kind`, `sniff`,
  `Overrides`, `Reading`, `Preview`, `preview`.
- `rust/crates/lakehouse-api/src/routes/uploads.rs`: `check_file` (~480,
  where a workbook is refused today with `WORKBOOK`), `create` (~512),
  `storage_key` (~383), `overrides_of` and `preview` (~895, ~929),
  `parse_ingest_request` (~1022), `run_config` (~1170), `ingest` (~1259),
  the fixed sentences (~156-230).
- `rust/crates/lakehouse-api/src/upload_store.rs`: reading and writing the
  stored object.
- Console: `src/lib/uploads.ts` (`fileKindProblem`), `upload-file-page.tsx`,
  `upload-check-step.tsx`, `src/services/contracts/uploads.ts`,
  `clients/uploads.ts`.
- Fixtures: `ops/fixtures/uploads/`. Gate: `ops/g9/upload_test.py`.

## 5. Tasks

One task per commit.

### X1 — Read a workbook (API, pure)

- A module beside `upload_parse.rs` that, from the bytes of an `.xls` or
  `.xlsx`: lists the sheets (name, visible or not), and converts one sheet
  to records of text by decision 2, refusing by decisions 4 and 6.
  No I/O, no HTTP.
- **Accept:** unit tests on small fixture workbooks committed under
  `ops/fixtures/uploads/` (one `.xlsx`, one `.xls`), covering each rule of
  decision 2, several sheets, a hidden sheet, an empty sheet, the cell
  cap, a file that is a zip but not a workbook, and a truncated file. Say
  how each fixture was made; if a valid `.xls` cannot be produced on this
  machine, stop and report.

### X2 — The routes

- `create`: a workbook is accepted and stored; the answer carries its
  sheets and the default. `.xlsm`, `.xlsb`, `.ods` and other zips keep a
  refusal with a reason.
- `preview`: takes `sheet` (default as decision 3); for a workbook the
  encoding and delimiter overrides are refused or ignored, whichever the
  developer finds consistent with the route (say which); the answer says
  which sheet was read.
- `ingest`: takes `sheet`; converts, stores the CSV beside the original,
  launches the existing job on it; the audit event and the run
  configuration name the sheet. Deleting an upload removes both objects.
- **Accept:** route tests in `tests/upload_routes.rs` for each of the
  above and each refusal; the existing CSV tests unchanged.

### X3 — The console

- `fileKindProblem` lets `.xls` and `.xlsx` through and still refuses the
  rest; the hint and the `accept` list say CSV, TSV, `.xls`, `.xlsx`.
- The Check step shows a sheet picker for a workbook, hides the encoding
  and delimiter controls, keeps the header-row control, and says that
  every column is loaded as text and that dates are written as ISO dates.
- **Accept:** unit and component tests; existing tests updated, each
  changed assertion listed.

### X4 — Gate and documents

- `ops/g9/upload_test.py` also uploads the `.xlsx` fixture and checks the
  loaded rows.
- `CHANGELOG.md`; `docs/core/features/upload-file.md` (what is accepted,
  the conversion rules, the limits, new checklist rows);
  `docs/adr/0014-file-upload.md` gains the amendment of section 3.

## 6. Working on this machine

- Work only in `/home/hv/lakehouse-merge` on `feat/upload-excel`. Never
  touch `/home/hv/lakehouse` or `/home/hv/lakehouse-uiux`. Never read any
  `.env`.
- This machine crashes when Rust test binaries are built. Allowed: `cargo
  fmt --check`; `cargo clippy --workspace --all-targets --all-features --
  -D warnings` with `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
  CARGO_BUILD_JOBS=2`; `cargo deny check` and `cargo audit` if installed.
  Not allowed: `cargo test`, `cargo build`, anything that links test
  binaries. The Rust tests first run in CI; every commit body and the
  handoff say so.
- No docker commands. No prettier. `df -h /` before a build; stop under
  12 GB free.
- Console and Python checks run normally: `bun run typecheck && bun run
  lint && bun run test`; the two `ops/lint` scripts; `python3 -m
  py_compile` on changed scripts.
- Commits end with `Co-Authored-By: Claude Sonnet 5.5
  <noreply@anthropic.com>`. Do not push.

## 7. Acceptance checklist (product owner)

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Choose an `.xlsx` with two sheets | Accepted; the Check step lists both sheets, the first chosen | |
| 2 | Pick the other sheet | The preview shows that sheet's rows | |
| 3 | Look at a date and a number column | Dates as `YYYY-MM-DD`, numbers without display formatting | |
| 4 | Load it | A raw table with the sheet's rows; every column text | |
| 5 | Choose an `.xls` (old format) | The same | |
| 6 | Choose an `.xlsm` or a password-protected workbook | Refused, with the reason | |
| 7 | Choose a CSV | As before | |

## 8. Handoff (developer)

## 9. Review (planner)
