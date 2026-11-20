# BI-55, the rest: an Excel text cell holds its text without the apostrophe

Backlog task `BI-55` (P3, Dashboards, In Progress). QA case: `LH-BI-1-08` (F1). No other QA
case is fixed here.

Base: `main` at `c7cd01a76` (every `file:line` below was read there). Branch:
`bi-55-chart-labels`. If `main` moves before the build starts, **re-find an anchor by the
text quoted with it, not by its line number**. Pull request #527 moves most of these lines;
section "Shared files with pull request #527" says where they go.

Planner and reviewer: Claude Opus. Developer: `Hy4 flash` (OpenCode). The developer writes
product code and tests and edits documents only where a task says so. If an anchor, a
signature or a quoted text below does not match the code, stop and report the mismatch in
the handoff; do not improvise. Never change an expected value to get a test green (rule 2).

## Why

QA (`LH-BI-1-08` F1, round of 2026-11-02 to 11-12): in a dashboard tile's Excel download, a
text that starts with `=` is a text cell (the sheet has no formula element) and still shows
the guarding apostrophe as part of the cell: the reader sees `'=1+1`, not `=1+1`. The
apostrophe is the CSV guard against formula injection (`SEC-17`, OWASP). In a CSV file it is
needed: a text file has no cell types, so a spreadsheet decides what `=1+1` is when it opens
the file. In an `.xlsx` file it is not: the export writes every text with `write_string`,
which stores a typed string cell, and a spreadsheet never evaluates a string cell. There the
apostrophe protects nothing and changes the data.

## What already shipped, and what is dropped

`BI-55` listed eight findings. Read against `main` at `c7cd01a76`:

| Finding | QA case | State | Where |
| --- | --- | --- | --- |
| An average shows no decimals | `LH-BI-6-04` | Shipped | #503 T1 (`de4ba2bdb`); alerts and sums in `BI-57` (#514, #519) |
| A year column reads "2,020 – 2,020.5" | `LH-BI-2-02` | Shipped, **dropped here** | #503's T2 was undone in review; fixed as `BI-57` (#514: `24c4fc3a9`, `96236aa81`). `BI-57` is Done. The backlog row's "year bins still open" is out of date and is corrected in this commit |
| A donut's legend is cut off | `LH-BI-6-39` | Shipped | #503 T8 (`1cf81018d`) |
| The Rows table has no label for empty/missing | `LH-BI-6-38` | Shipped | #503 T9 (`f6e0426ee`) |
| Records panel does not name the question | `LH-BI-13-04` | Shipped | #503 T10 (`35fdb75d4`) |
| A joined column reads `table__column` | `LH-BI-7-04` | Shipped | #503 T5 (`56d83a530`) |
| Compact currency cannot be chosen; "IDR" | `LH-BI-1-06` | Shipped | #503 T4 (`931bec10c`) |
| Excel shows the guarding apostrophe | `LH-BI-1-08` | **Open: this plan** | #503's T3 was held for the owner (`docs/superpowers/plans/2026-11-12-qa-round-dashboards-p3.md`, D11, T3, open question 1); not built |

Re-checked from the code at `c7cd01a76`: the apostrophe still reproduces. `write_xlsx_file`
writes the header through `csv_export::neutralise(label)` (`export.rs:568`) and a text cell
through `csv_export::cell_text(Some(other))` (`export.rs:618`), which calls `neutralise`
(`csv_export.rs:25-26`). The test `xlsx_keeps_numbers_as_numbers_with_the_charts_number_format`
pins the apostrophe today (`export.rs:953-956`).

## Parallel work checked (2026-11-20)

`gh pr list --state open`:

- **#527** (`fix/bi-export-safety`, `BI-27`/`BI-28`/`BI-30`/`BI-34`, mtharreqq): touches
  `csv_export.rs`, `csv_safe.rs` and `export.rs`, the same files and the same function as
  this plan. It does **not** fix `LH-BI-1-08`: at its head `f8b07b4ba` the workbook header
  still calls `csv_export::neutralise(label)` and a text cell still goes through
  `text_cell` → `csv_export::cell_text` → `neutralise`, and its own new test keeps the
  apostrophe on a text column in the workbook. It is ready, CI green, and its merge state
  is `DIRTY` (conflicts with `main`). See "Shared files with pull request #527".
- **#531**, **#541**, **#530** (dashboard UI): no file in common (`src/lib/style-draft.ts`,
  `routes/pdf_export.rs`). **#527**'s other files and **#552** (`Cargo.lock`) are not
  touched here. No other open PR fixes `LH-BI-1-08`.

## 1. Decisions

### Process

- **D1. One task per commit, in the order of section 2** (T1 is the Rust change with its
  tests; T2 is the documents). The commit body cites `BI-55` and `LH-BI-1-08`. The fix site
  carries a comment naming the case (rule 13), e.g.
  `// QA LH-BI-1-08 (BI-55, D3): an .xlsx text cell is a typed string ...`. Push after
  every commit; the planner opens a draft pull request after the first push so CI runs the
  full suites.
- **D2. No new route, no migration, no new permission, no stored shape, no dependency.**
  `POLICY_TABLE` and `tests/route_auth.rs` do not change. `rust_xlsxwriter` stays
  `=0.102.0` (`lakehouse-api/Cargo.toml:235`).
- **D2a. Assistant (assistant parity).** No tool, schema or prompt change, and
  `rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json` is not regenerated. Reason:
  the assistant's `export_chart` and `export_dashboard` only return a link to the console's
  own download route (`routes/ai/tools/export.rs:1-10`); the file is written by the same
  `write_xlsx_file` the menu uses, so the assistant's Excel file changes with this fix and
  no input or output of a tool changes. The dashboard page sends the same context. The
  standard request set (`ops/ai_eval/dashboard_cases.json`, `AI-15`) has no case that opens
  a workbook's cells, so no case is added or changed. The AI team reviews this paragraph.

### The rule

- **D3. An Excel text cell and an Excel header cell hold the text as it is, with no
  apostrophe** *(planner default, owner may change)*. This is #503's held D11, unchanged.
  The other choice: keep the apostrophe in Excel too and close `LH-BI-1-08` as intended
  (then nothing in section 2 is built and only the backlog row changes). The protection is
  the cell type: every text is written with `write_string` / `write_string_with_format`,
  never `write_formula` and never `write` (which guesses a type), and a test pins that the
  sheet holds no formula element for `=`, `+`, `-` and `@`.
- **D4. CSV and TSV keep the apostrophe.** `csv_export::neutralise`, `csv_export::cell_text`,
  `csv_cell` (`export.rs:254-276`), `csv_safe` and every CSV/TSV test stay as they are. The
  audit, usage and upload CSVs are not touched.
- **D5. The workbook's byte cap counts the text the cell holds**, now without the
  apostrophe (`xlsx_cell_bytes` and the header's bytes), as the module doc says ("the
  workbook counts the text of its cells", `export.rs:17-19`). A formula-looking cell costs
  one byte less than today.
- **D6. One match, not two.** The text of a value is computed once, in `csv_export`:
  `plain_text` holds the match; `cell_text` is `neutralise` over it for strings. No second
  copy of the match in `export.rs` (rule 4).

### Shared files with pull request #527

#527 restructures `write_xlsx_file`: the row writes move into `XlsxSink::write_rows`
(at `f8b07b4ba`: `export.rs:616-659`, the text cell at `:646-654`), a text cell's text comes
from a new `text_cell(value, numeric)` (`:340-348`, shared with `csv_cell`), and
`xlsx_cell_bytes` calls `text_cell` (`:594-599`). The header site (`:713-719`) and the
header's bytes (`:730-735`) keep their shape. `csv_export::neutralise` becomes a call to
`csv_safe::neutralise`.

The change of T1 is the same in both shapes: **at the Excel sites only**, the text is
`csv_export::plain_text(Some(value))` (no neutralise), whatever the column; `text_cell` and
`csv_cell` are **not** changed (they are the CSV path). Whichever of #527 and this branch
merges second rebases onto the other; the developer resolves a conflict hunk by hunk
(rule 9), keeps #527's batching and `text_cell`, and re-applies T1's four Excel sites. If
#527 merges before T1 is built, rebase on `main` first and build T1 on #527's shape.

## 2. Tasks

### T1. An Excel text cell holds its text without the apostrophe (LH-BI-1-08; BI-55, D3 to D6)

Anchors at `c7cd01a76`, all in `rust/crates/lakehouse-api/`:

- `src/csv_export.rs:6-13` (module doc, the formula guard), `:20-29` (`cell_text`),
  `:31-40` (`neutralise` and its doc, which says "the CSV writer below and the `.xlsx`
  writer (`export.rs`) both call it").
- `src/export.rs:23-25` (module doc: "formula neutralising is `csv_export::neutralise` for
  a CSV cell, a tab-separated cell and an Excel text cell alike"), `:520-528`
  (`xlsx_cell_bytes`), `:564-570` (the header cells,
  `write_string_with_format(0, col, csv_export::neutralise(label), &bold)`), `:575-579`
  (the header's bytes, `csv_export::neutralise(label).len()`), `:617-625` (a text cell,
  `let text = csv_export::cell_text(Some(other));` then `sheet.write_string(r, col, text)`).
- Tests: `src/export.rs:920-957`
  (`xlsx_keeps_numbers_as_numbers_with_the_charts_number_format`; comment at `:941`, last
  assertion `:953-956`), `:982-999` (`a_workbook_past_the_byte_cap_is_cut`), test helpers
  `source` `:692-700` (columns `name` String, `amount` Int64; `make` is called with
  1, 2, 3, ...), `options` `:702-717`, `read_all` `:719-724`;
  `tests/query_download.rs:485-527`
  (`a_cell_that_starts_a_formula_is_guarded_in_every_text_format`; doc `:485-487`, the
  workbook half `:521-527`), `:567-580` (`workbook_text`).

Change:

1. `src/csv_export.rs`: add
   `pub(crate) fn plain_text(value: Option<&Value>) -> String` with the match of today's
   `cell_text` and no `neutralise`: `None | Some(Value::Null)` → `String::new()`, a number
   → `n.to_string()`, a bool → `b.to_string()`, a string → `s.clone()`, anything else →
   `other.to_string()`. Doc: "A cell's text as it is, with no formula guard: the text an
   `.xlsx` string cell holds (QA `LH-BI-1-08`, `BI-55`)." Then `cell_text` becomes: for
   `Some(Value::String(_))` and the "anything else" arm, `neutralise(&plain_text(value))`;
   for null, number and bool, `plain_text(value)`. Its output must be byte-identical to
   today's for every input. Edit `neutralise`'s doc: the comma file and the tab-separated
   file call it; an `.xlsx` text cell does not, because it is a typed string cell that a
   spreadsheet never evaluates (QA `LH-BI-1-08`, `BI-55`). Edit the module doc's sentence
   "This is the only place the export neutralises cells" so it does not claim the workbook
   is neutralised. `neutralise`'s body is **not** changed.
2. `src/export.rs`, four Excel sites, nothing else:
   - header cell (`:568`): `write_string_with_format(0, col, label.as_str(), &bold)`;
   - header bytes (`:578`): `label.len() as u64`;
   - `xlsx_cell_bytes` (`:526`): `csv_export::plain_text(Some(value)).len() as u64`;
   - text cell (`:618`): `let text = csv_export::plain_text(Some(other));`, with the
     comment `// QA LH-BI-1-08 (BI-55, D3): an .xlsx text cell is a typed string cell that a
     spreadsheet never evaluates, so it holds the text as it is; the CSV keeps the
     apostrophe (csv_export::neutralise).` Keep `write_string`; never `write_formula`, never
     `write`.
   Module doc `:23-25`: formula neutralising is `csv_export::neutralise` for a CSV cell and
   a tab-separated cell; an Excel text cell is a string cell, never a formula, and holds
   its text as it is (`BI-55`, QA `LH-BI-1-08`).
3. CSV and TSV code paths are not touched (`csv_cell`, `csv_export::line`, `tsv_line`).

Checks (each new or changed assertion fails on `c7cd01a76`; say in the handoff which
assertion failed before the fix, with its message):

- New, in `src/export.rs` tests:
  **`a_text_that_starts_like_a_formula_is_a_string_cell_in_the_workbook_with_no_apostrophe_and_no_formula`**.
  `labels = HashMap::from([("name".to_owned(), "=Name".to_owned())])`, `formats` empty.
  `make` (a non-capturing closure, so it coerces to `fn(usize) -> Vec<Value>`) returns
  `vec![json!(TEXTS[i - 1]), json!(i)]` with
  `const TEXTS: [&str; 5] = ["=1+1", "+SUM(1,2)", "-2+3", "@SUM(1)", "=HYPERLINK(\"http://example.invalid\",\"x\")"];`
  and `source(5, make)`. Write `FileFormat::Xlsx`, `Values::Raw`, cap 10. Read
  `xl/worksheets/sheet1.xml` and, if present, `xl/sharedStrings.xml` (as `xlsx_keeps_numbers_as_numbers_with_the_charts_number_format` does)
  into `sheet` and `shared`; `all = sheet + shared`. Assert: `w.rows == 5`; `sheet`
  contains neither `<f>` nor `<f `; `sheet` contains `t="inlineStr"` or `t="s"`; `all`
  contains each of `=1+1`, `+SUM(1,2)`, `-2+3`, `@SUM(1)`, `=HYPERLINK(` and `=Name`; `all`
  contains none of `'=1+1`, `'+SUM`, `'-2+3`, `'@SUM`, `'=HYPERLINK`, `'=Name`, and none of
  `&apos;=`, `&apos;+`, `&apos;-`, `&apos;@` (the writer may escape the apostrophe).
- New, in `src/export.rs` tests:
  **`a_formula_looking_text_costs_its_own_bytes_in_the_workbook_cap`**: `make` returns
  `vec![json!("=x"), json!(i)]`; labels and formats empty; `write_export_with_byte_cap` (`export.rs:330`)
  over `source(10, make)`, `FileFormat::Xlsx`, `Values::Raw`, cap 10, byte cap
  `Some(10 + 3 * 3)` (header `name` + `amount` is 10 bytes; a row is `=x` 2 bytes plus a
  one-digit number). Assert `w.cut` and `w.rows == 3`. (Today each row counts 4 bytes,
  so `w.rows` is 2.)
- Changed, `src/export.rs:941` and `:953-956`
  (`xlsx_keeps_numbers_as_numbers_with_the_charts_number_format`): the comment becomes
  "The formula-looking text is a string cell holding the text as it is (QA `LH-BI-1-08`).";
  the last assertion becomes: `sheet` or `shared` contains `=HYPERLINK`, and neither
  contains `'=HYPERLINK` nor `&apos;=HYPERLINK`. The `!sheet.contains("<f>")` assertion
  stays.
- Changed, `tests/query_download.rs:521-527`: the workbook's `text` contains `=1+1`, and
  contains none of `'=1+1`, `&apos;=1+1`, `<f>`, `<f `. The CSV half (`:514`) and the TSV
  half (`:519`) stay byte for byte. The doc comment (`:485-487`) becomes: a cell that would
  run as a formula gets a leading apostrophe in the comma and tab files; in the workbook it
  is a string cell holding the text as it is, never a formula (`SEC-17`; `BI-55`, QA
  `LH-BI-1-08`); a number in a number column is untouched.
- Unchanged and passing (do not edit): `csv_export::tests::*`,
  `export::tests::a_quoted_64_bit_integer_is_a_number_and_a_formula_is_neutralised`,
  `export::tests::a_negative_quoted_integer_is_written_unchanged_and_text_is_still_neutralised`,
  `a_tab_separated_file_has_the_bom_the_tabs_and_the_line_ends_of_the_comma_one` (`:886`), `export::tests::a_workbook_past_the_byte_cap_is_cut`
  (its labels and cells hold no formula character, so 10 + 2 per row is unchanged), and
  every test in `tests/dashboard_export.rs`, `tests/audit_search.rs` and
  `tests/usage_routes.rs` (they read CSV files).

Commands for this commit:

```bash
cd rust && cargo fmt --check
cargo clippy -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-api --lib export::
cargo test -p lakehouse-api --lib csv_export::
cargo test -p lakehouse-api --test it query_download::
cargo test -p lakehouse-api --test it sec11_guard::
```

Commit: `fix(export): an Excel text cell holds its text without the apostrophe (BI-55, LH-BI-1-08)`.

### T2. The documents say the rule (BI-55)

No code. Edit exactly these lines:

- `docs/core/features/export-pdf-excel.md`, Decisions table row 7 (`:43`): replace its last
  sentence "Cells that start like a formula are neutralised in both (as the CSV export does
  today)." by
  `A text that starts like a formula is never run: in CSV it gets a leading apostrophe; in Excel it is a text cell and holds the text as it is (changed 2026-11-20, \`BI-55\`, *(proposed, not yet signed)*).`
  and its Default cell `Planner` becomes `Planner; changed: planner default (QA \`LH-BI-1-08\`)`.
- Same page, "Limits to tell a customer" (`:54-59`), a new last line:
  `- An Excel file keeps a text that starts with =, +, - or @ as a text cell, without an apostrophe. If the reader saves that file as CSV with their own program, the new file is not guarded. *(proposed, not yet signed; \`BI-55\`)*`
- `docs/core/features/query-studio-results.md`, Decisions table row 6 (`:64`): after
  "with the formula guard dashboard exports use (`csv_export::neutralise`)." insert
  ` In Excel a text cell holds the text as it is, without the apostrophe: a text cell is never run (\`BI-55\`, *(proposed, not yet signed)*).`
- `CHANGELOG.md`, under `## [Unreleased]`, `### Fixed` (`:104`), a new first bullet:
  `- An Excel download (a dashboard tile, Query Studio) shows a text that starts with =, +, - or @ as it is, without the guarding apostrophe; the cell is a text cell and is never run as a formula. CSV and TSV downloads keep the apostrophe (backlog \`BI-55\`, QA \`LH-BI-1-08\`).`

The developer does not edit `docs/core/BACKLOG.md` or `PRODUCT.md`.

Commit: `docs(features): an Excel text cell holds its text as it is (BI-55, LH-BI-1-08)`.

## 3. PR slicing

One pull request, two commits, branch `bi-55-chart-labels` (this plan's commit is its first).
Languages: Rust (`lakehouse-api` only) and Markdown. No TypeScript, Python or compose
change, so those checks are not run.

## 4. Verification before handoff

Run once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check \
  && cargo clippy --workspace --all-targets --all-features -- -D warnings \
  && cargo test -p lakehouse-api
git status   # nothing generated staged
```

The full `cargo test --workspace` is CI's job on the draft pull request (it cannot run all
API test files on the development machines in time); the handoff quotes the counts of the
commands it ran and writes *not verified* for the rest (rule 7). Use the shared
`CARGO_TARGET_DIR`; never `cargo clean`.

## 5. Out of scope

- **Text format on Excel text cells** (`num_format("@")`), so that a reader who edits a
  `=1+1` cell and presses Enter keeps it as text. Not checked in a spreadsheet; a separate
  task if the owner wants it.
- **The CSV and TSV apostrophe** and `csv_safe`'s rule (#527, `BI-30`).
- **The `Note` sheet #527 adds**: it is written by `write_string` with a fixed sentence and
  has no user text; nothing to change.
- **The workbook a reader re-saves as CSV** is not guarded; said on the feature page.

## Open questions for the owner

1. **Excel without the apostrophe** (D3; #503's open question 1, never answered). Planned:
   an Excel text cell holds the text as it is; CSV and TSV keep the apostrophe.
   Recommendation: as planned (the cell is typed text and is never evaluated; the
   apostrophe changes the data). The other choice: keep the apostrophe in Excel and close
   `LH-BI-1-08` as intended; then T1 and T2 are not built.

## Handoff

Developer: `Hy4 flash` (OpenCode), branch `bi-55-chart-labels` off `a7cce7609` (the
plan commit on `c7cd01a76`). All anchors were re-read by their quoted text at that
head; every one matched. No mismatch with the plan.

### Commits (each pushed)

1. `0a2f2730b` — T1, `fix(export): an Excel text cell holds its text without the apostrophe (BI-55, LH-BI-1-08)`.
   `csv_export::plain_text` is the single match; `cell_text` is byte-identical to the old
   output; the four Excel sites use `plain_text` (header, header bytes, cell bytes, text
   cell); CSV/TSV untouched.
2. `948bc5738` — T2, `docs(features): an Excel text cell holds its text as it is (BI-55, LH-BI-1-08)`.
   Only the three documents named in T2 changed; `BACKLOG.md` and `PRODUCT.md` untouched.

### New and changed assertions fail on the old code

Run with the new tests added but the source unchanged, at `a7cce7609`:

- `export::tests::a_text_that_starts_like_a_formula_is_a_string_cell_in_the_workbook_with_no_apostrophe_and_no_formula`
  → `'=1+1 in <?xml … <t>'=1+1</t> …` (assertion at `export.rs:1025`).
- `export::tests::xlsx_keeps_numbers_as_numbers_with_the_charts_number_format`
  → assertion at `export.rs:953` failed with the sheet showing `<t>'=HYPERLINK("http://x")</t>`.
- `export::tests::a_formula_looking_text_costs_its_own_bytes_in_the_workbook_cap`
  → `assertion left == right failed: left: 2, right: 3` at `export.rs:1045` (old rows 2, new 3).
- `query_download::a_cell_that_starts_a_formula_is_guarded_in_every_text_format`
  → `the workbook cell holds the text, not a guarded formula: … <t>'=1+1</t> …` at
  `tests/query_download.rs:527`.

### Verification on the final commit (foreground, counts as run)

`CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-6` (shared, outside the checkout);
no `cargo clean`. TypeScript, Python and compose were not touched and were not run.
No `cargo test --workspace`/workspace clippy — CI's job, per the developer brief.

- `cd rust && cargo fmt --check` — clean.
- `cargo clippy -p lakehouse-api --all-targets --all-features -- -D warnings` — clean, finished in 50.82 s.
- `cargo test -p lakehouse-api --lib export::` — 62 passed, 0 failed, 3,699 filtered out.
- `cargo test -p lakehouse-api --lib csv_export::` — 6 passed, 0 failed, 3,755 filtered out.
- `cargo test -p lakehouse-api --test it query_download::` — 23 passed, 0 failed, 1,751 filtered out.
- `cargo test -p lakehouse-api --test it sec11_guard::` — 4 passed, 0 failed, 1,770 filtered out.
- `cargo test -p lakehouse-api` (plan §4) — first attempt with default threads: 3,716 passed,
  27 failed, 18 ignored; every failure was `start the ClickHouse testcontainer:
  Client(CreateContainer(RequestTimeoutError))` in `registry_clash`/engine tests. Retry with
  `-- --test-threads=4`: the lib binary passed (re-run alone: 3,743 passed, 0 failed,
  18 ignored in 740.45 s); the `it` binary reached 1,768 passed, 1 failed, 5 ignored —
  the one failure `sql_lex_corpus::the_guard_agrees_with_a_real_clickhouse_on_every_corpus_row`
  again `CreateContainer(RequestTimeoutError)`. Docker is up but container creation times
  out under parallel agent load; none of the failures is in an export test. Those two
  container-start cases are **not verified** here; CI runs them.
- `git status` before each commit: only the intended files modified; no generated files
  staged. After the final commit the tree is clean.
- Disk pressure during the build: the shared target's regenerable `debug/incremental/`
  scratch (4.2 GiB) was removed to let the link finish; no `cargo clean`, no compiled
  dependency cache wiped.

## Review

### Round 1 — planner review, 2026-11-20, at `647fe2e92`

**No finding.** The four Excel sites write the text as it is
(`csv_export::plain_text`), through `write_string*`, which always makes a
typed string cell; CSV and TSV keep `neutralise`. The new test asserts the
formula-looking texts are in the sheet with no apostrophe and no formula
element; the changed assertions fail on the old code. The handoff's one
`it` Docker timeout and two container cases are left to CI.
