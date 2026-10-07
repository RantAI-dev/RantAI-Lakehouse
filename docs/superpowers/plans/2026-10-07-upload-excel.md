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

Branch `feat/upload-excel` off `fix/upload-large-files` (64f391e). Not pushed.
One PR slice (X1 to X4).

### Commits

| Task | Commit | What |
| --- | --- | --- |
| X1 | `732e0c6` | `upload_workbook.rs`, `calamine` 0.36, fixtures in `ops/fixtures/workbooks/` and `ops/fixtures/uploads/converted_sheet.*` |
| X2 | `349227d` | routes (`create`, `preview`, `ingest`, `delete`), `UploadStore::get_all`, route and unit tests |
| X3 | `1dd235d` | console: `fileKindProblem`, hint and `accept`, sheet picker, contract and client |
| X4 | this commit | `ops/g9/upload_test.py`, `CHANGELOG.md`, feature page, ADR 0014 amendment, this section |

### Commands run, with results

Rust (in `rust/`, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target`,
`CARGO_BUILD_JOBS=2`, `df -h /` showed 26 to 27 GB free before each):

- `cargo fmt --check`: clean (X1, X2 final state).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: clean
  on the X2 final state (this type-checks and lints the new unit tests and
  `tests/upload_routes.rs` without linking them). No test count: no test was run.
- `cargo deny check`, `cargo audit`: **not installed** on this machine, so not
  run. Licences were checked by hand: `calamine` 0.36.1 MIT; its new transitive
  crates `atoi_simd` 0.18.1, `codepage` 0.1.3, `debug_unsafe` 0.1.4,
  `fast-float2` 0.2.4, `typed-path` 0.12.3 (each MIT OR Apache-2.0 or
  Apache-2.0 OR MIT), `zip` 8.6.0 (MIT), `zopfli` 0.8.3 (Apache-2.0); all are in
  `deny.toml`'s allow list. Advisories **not checked**.

Console (repo root):

- `bun run typecheck`: clean.
- `bun run lint`: 0 errors, 6 warnings (the same 6 on the base commit).
- `bun run test`: 830 pass, 1 skip, 0 fail (831 tests, 92 files).
- `python3 ops/lint/check_intra_package_imports.py` and
  `python3 ops/lint/check_bare_iceberg_count.py`: both OK.
- `python3 -m py_compile` on `ops/g9/upload_test.py` and
  `ops/fixtures/workbooks/make_workbooks.py`: clean.
- Stand-in for the Python fixture test: `csv.reader` (the load's dialect) over
  `converted_sheet.csv` gives `converted_sheet.expected.json`'s columns and rows
  (28 records). `pytest`/`dagster` are not installed here, so
  `dagster/dispar_orchestrate/test_file_ingest.py` was **not run**; it picks up
  the new fixture by its glob.

### Not verified

- **Every Rust test** (`upload_workbook.rs` unit tests, `routes/uploads.rs` unit
  tests, `tests/upload_routes.rs`, `upload_parse.rs`'s fixture tests with the
  new `converted_sheet` entry). They were not run: this machine may not link
  test binaries. First run is CI.
- That `calamine` reads the `.xls` that `xlwt` wrote and returns what the tests
  expect (dates, error cells, hidden sheet, merged cells). Never executed.
- `cargo deny` and `cargo audit` (not installed).
- `ops/g9/upload_test.py` (the gate): not run, no stack.
- No docker command, no browser check of the console.

### Fixtures, and how each was made

All values invented. `ops/fixtures/workbooks/make_workbooks.py` regenerates
every file; it needs `openpyxl` (3.1.5 was already installed system-wide, used
as is) and `xlwt` (not installed: its 1.3.0 wheel was `pip download`ed into
`/home/hv/.cache/lakehouse-deploy/xlwtlib` and unpacked there, nothing was
installed anywhere, run with `PYTHONPATH=.../xlwtlib/pkg`). No LibreOffice.

- `stock.xlsx` (openpyxl) and `stock.xls` (xlwt, BIFF8): sheets `Stock`,
  `Quirks`, `Offset`, `Hidden notes` (hidden), `Empty`.
- `formula.xlsx`: openpyxl cannot store a formula's result, so the script
  patches `sheet1.xml` after writing (adds `<v>3</v>`, a `t="str"` result and a
  `t="e"` result) and fails if a patch does not apply.
- `date1904.xlsx`: openpyxl with `wb.epoch = CALENDAR_MAC_1904`; serial 44462
  was checked in the XML.
- `plain.zip`: Python `zipfile`.
- The encrypted workbook has no fixture (no tool here can encrypt): tests use
  synthetic bytes (OLE magic plus the stream name `EncryptedPackage` in
  UTF-16LE), which exercise the detection only.
- `uploads/converted_sheet.csv` and `.expected.json`: written by hand from the
  conversion rules (not generated by running the converter).

### How a cell becomes text, where the plan left room

Number: `Display` for `f64` (shortest round trip), `{:e}` below 1e-6 and from
1e21. Examples: `1.0` -> `1`; `0.1` -> `0.1`; `0.1+0.2` -> `0.30000000000000004`;
`1e21` -> `1e21`; `1e-7` -> `1e-7`; `-0.0` -> `0`; a cell `25%` holding 0.25 ->
`0.25`; `1234.5` formatted `#,##0.00` -> `1234.5`. Date `2025-09-24`; date-time
`2025-09-24 13:30:05`; time `13:30:00`; `[h]:mm:ss` 1.5 days `36:00:00`;
`#DIV/0!` -> `#DIV/0!`; boolean `true`; formula with no stored result: empty.
Milliseconds append `.mmm` only when non-zero. A date-formatted value below 1
is a time of day (also in 1904 workbooks, where 0 is also 1904-01-01); a value
negative or past about year 9990 is written as the number.

### Departures from the plan

1. **The sheet is not in the run configuration.** The job's `config_schema` is
   closed, Dagster would refuse an unknown key, and the job must not change. The
   sheet is in the audit event and in the row's `parse_options` (so the console's
   "Try again" sends it back). No migration.
2. **"Is a workbook" is decided by name AND first bytes, not by the stored name
   alone** (decision 5): `sap_report_utf16.xls` is text named `.xls`, and an
   existing route test uploads CSV bytes as `stock.xlsx`. An upload named
   `.xls`/`.xlsx` costs one 8-byte range read in preview-adjacent paths
   (`ingest`); other names cost none.
3. **Ingest parses the body after reading the row** (what the body must carry
   depends on the upload: a workbook needs no encoding and delimiter). The
   handler order is now row, workbook check, body, settle; a 400 for a bad body
   still asks nobody but Postgres.
4. **`encoding` and `delimiter` are validated then ignored** on a workbook
   preview, and not read at all on a workbook ingest.
5. **An empty sheet is refused at ingest** with a new fixed sentence ("That
   sheet is empty, so there is nothing to load."), not left to the job's
   "no rows" reason; the preview of it is empty, as for an empty file.
6. Preview reads the first 2,000 records of the converted sheet (the workbook is
   read whole, the preview text is a prefix), so a header row past that shows no
   columns. Stated on the feature page.
7. Records start at the sheet's first filled cell (leading empty rows and
   columns are dropped; formatted-only cells do not count), so the header-row
   number is relative to that. Stated on the feature page and in the Check step.
8. Existing assertions changed: `a_file_must_not_be_empty_over_the_cap...`,
   `a_file_is_judged_by_its_first_bytes_not_by_its_name` (new sentence; the
   `old.xls` OLE case now uses `old.dat`, because `.xls` is accepted by name),
   the `parse_ingest_request` unit tests (new `workbook` argument, `sheet: None`),
   `PreviewQuery` helper (`sheet`), and the console tests listed in the X3
   commit body.
9. `calamine`'s `worksheet_range` would allocate the whole bounding box of a
   hostile sheet before the cap could see it; for `.xlsx` the cells are
   streamed and the cap checked per cell. For `.xls` the library builds the sheet
   first (bounded by the format at 65,536 x 256 cells, about 0.5 GB worst case).

### Where CI is most likely to fail, in order

1. `calamine`'s reading of the `xlwt` file or my guesses about its output in
   `the_quirks_sheet_converts_to_the_pinned_text_in_both_formats` (duration,
   error code 42, merged cell, percent); then the 1900-02-29 and `1e-7`
   expectations in `upload_workbook.rs`.
2. `tests/upload_routes.rs`: the new workbook tests (bucket fake and a
   whole-object GET without a range header; the cut/garbage `.xls` refusals).
3. `a_file_cut_anywhere_never_panics` (a parser that loops or allocates on cut input).
4. `cargo deny` / `cargo audit` on the new crates (not run here).
5. `converted_sheet` in the Python fixture test (padding cell `  padded  `).
6. The console is the least likely: all of it ran.


## 9. Review (planner)
