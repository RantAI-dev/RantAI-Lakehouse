# QA round, Dashboards (P3): numbers and labels on charts, and the PDF's layout

Fixes for two backlog tasks opened by the QA round of 2026-11-02 to 11-12 on staging:
`BI-55` (P3, under `BI-1`, `BI-2`, `BI-6`, `BI-7`, `BI-13`) and `BI-53` (P2, under `BI-1`).
QA cases: `LH-BI-2-02`, `LH-BI-6-04`, `LH-BI-6-39`, `LH-BI-6-38` (retest 2026-11-12),
`LH-BI-13-04`, `LH-BI-7-04`, `LH-BI-1-08`, `LH-BI-1-06` (`BI-55`); `LH-BI-1-01`,
`LH-BI-6-22`, `LH-BI-6-23` (retest 2026-11-11) (`BI-53`).

Base: `main` after pull request #486 is merged; anchors read at `7ac20f8be`. Nobody starts
this slice before that merge. The backlog rows were read at `657c2502`; #486 moved their
line numbers. If #486 gets more commits before it merges, **re-find an anchor by the text
quoted with it, not by its line number**.

Branch: `fix/qa-round-dashboards-p3`. One draft pull request for the whole slice, opened by
the planner as soon as the first commit is pushed, so CI runs the full suites.

Another slice, `dashboards-p2` (`BI-49`, `BI-50`, `BI-51`, `BI-52`, `BI-54`), is built at
the same time by another developer on `fix/qa-round-dashboards-p2`. The files both slices
edit are listed in "Shared files with dashboards-p2" at the end of section 1. In those
files this plan's edit is small and local; do not reformat or move anything else in them.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer writes
product code and tests and edits documents only where a task says so. If an anchor, a
signature or a quoted text below does not match the code, stop and report the mismatch in
the handoff; do not improvise.

Slices: this plan is **S1** = all of `BI-55`, the two causes of `BI-53` that reading shows,
and the investigation tasks of `BI-53`. **S2** (a later plan) = whatever makes the PDF
faster, chosen from the numbers S1 writes down. S1 changes nothing for speed.

What QA saw:

- `LH-BI-2-02` F1: a year column picked as the dimension starts on "Automatic (about 10
  bins)" and its bars read "2,020 – 2,020.5" (a thousands separator and a decimal on a year).
- `LH-BI-6-04`: the "Other" bar of an average shows 1466; the same average in Query Studio
  is 1466,18. "The average is shown without decimals."
- `LH-BI-6-39`: a donut over a column of 140 values (top 20 kept): the legend is cut off.
  (Also "the preview is small": not in this slice, section 4.)
- `LH-BI-6-38`, retest 2026-11-12: on the chart NULL reads "(No value)" and the empty string
  "(Empty)" (since #453); in the "Rows" table the two rows have no label.
- `LH-BI-13-04` F1: the records panel opened from a bar and the one opened from the line of
  the same chart look the same; nothing says which question the records belong to.
- `LH-BI-7-04`: the record list and the CSV of a raw table name a joined column
  `zz_qa_hotel_1105__kamar`, not `table.column` as decision 3 of the feature page says.
- `LH-BI-1-08` F1: in the Excel file a text that starts with `=` is a text cell (no formula
  element) and still shows the guarding apostrophe as part of the cell (`'=1+1`).
- `LH-BI-1-06` F1: "the compact currency style 'Rp 1,2 M' cannot be chosen together with
  Currency; the symbol reads 'IDR'" (the deployment's number locale was `en-US`).
- `LH-BI-1-01`: PDF, A4 landscape, 7 pages, one tile per page, about 45 seconds. F1: tiles
  stay at half width, not across the page. F2: the dark background does not cover pages 4
  and 7 (white bands). F3: about 45 seconds.
- `LH-BI-6-22`, `LH-BI-6-23`, retest 2026-11-11: in a 13-page PDF of 26 charts the points of
  the scatter and the bubble chart (about 555 to 566 thousand) are cut at the card's right
  edge, "the X axis stops at 500K"; on screen the axis reaches 600K and nothing is cut.

## 1. Decisions

### Process

- **D1. One task per commit, in the order of section 2** (Rust first, then the console, then
  the PDF, then the documents). Each commit body cites its QA case ID and backlog ID. Each
  fix site carries a short comment naming the case (rule 13), e.g.
  `// QA LH-BI-6-04 (BI-55): …`. Push after every commit.
- **D2. No new route, no migration, no new permission, no change to a stored shape.**
  `POLICY_TABLE` and `tests/route_auth.rs` do not change. The only new field on the wire is
  additive: `whole` inside a tile's `bins` entry (D7).
- **D3. Assistant (assistant parity).** No tool, schema or prompt changes, and
  `rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json` is not regenerated. Reason:
  a PDF's layout, a legend, a label and a file's cell are not things a tool sets. The two
  tasks that touch what the assistant builds change no shape it fills in: T1 changes the
  statement the builder writes for an average, a minimum and a maximum (the assistant sends
  the same `aggregate` words, `registry.rs:177`), and T4 keeps `currencyDisplay` at
  `symbol | code` (`registry.rs:470`) and the `compact` style with a `currency`, which the
  schema already takes. No case of the standard request set (`ops/ai_eval/dashboard_cases.json`)
  expects a rounded average (read: no case asserts a value of an `avg`), so no line of it
  changes. `lakehouse_bi::specs` and `src/lib/dashboard-specs.ts` (the drift guard
  `specs_match_typescript`) are not touched: the built-in charts use `sum` only, and sums do
  not change (D4).

### BI-55: numbers and labels

- **D4. Rounding per aggregate (finding 2).** Today the builder writes `round(<agg>(x))`
  for every aggregate but `count` at six places, so an average of 1466.18 is stored and
  drawn as 1466. New rule, in one function (`Aggregate::rounded`, T1):
  - `count`: `count()`, a whole number (unchanged);
  - `sum`: `round(sum(x))`, a whole number (unchanged, see below);
  - `avg`, `min`, `max`: `round(<agg>(x), 6)`. Six decimals is enough for a percent shown
    with four decimals and cuts the tail of a binary fraction (1466.1833333333334). For a
    whole-number column `round(x, 6)` is the number itself in ClickHouse, so a minimum or a
    maximum of whole numbers stays whole.
  - `sum` is **not** changed in this slice: 66 pinned statements, the built-in dashboards
    and the drift guard hold `round(sum(`, and dashboards-p2 adds more. A sum of a decimal
    column therefore stays rounded to a whole number; this is named in section 4 with a
    follow-up line.
  - What the screen then shows (read in `src/lib/chart-axis.ts:33-45`, `value-format.ts`):
    with "As it is now" (no format) a number keeps about four significant digits, the rule
    of `decimalsFor`: 0.1466 reads "0.147", 14.66 "14.66", 146.62 "146.6", 1466.18 "1,466".
    With Format > Number and decimals 2 it reads "1,466.18" (today "1,466.00"). A
    single-number card with no format keeps whole numbers (`tile-body.tsx:137`, `fmtInt`).
    `decimalsFor` and the card are not changed (section 4, open question 2). The Rows
    table and a CSV of the tile hold the stored value (1466.183333).
  - **A chart saved before this change.** A plain chart (no grain, no own filters, no
    measure with its own aggregate) is read with the statement it was saved with when no
    dashboard filter applies (`builder.rs:1710-1723`), so it keeps whole numbers until it is
    saved again; with a filter it is rebuilt and shows decimals. Stored statements are not
    rewritten and the rebuild rule is not widened here (open question 4).
- **D5. A year as the dimension (finding 1).** The builder knows a column only as `number`
  (`src/lib/time-grain.ts:17`; the server's `ColumnKind` is "never from the column's name",
  `filters.rs:465-466`): neither that it holds whole numbers nor how many values it has. So
  the rule is made where the data is read, in the same statement that reads the column's
  minimum and maximum:
  - automatic and counted bins of a column whose values (over the rows read) are all whole
    numbers are never narrower than 1;
  - the tile says so (`whole`), and a bin of width 1 over such a column is labelled by its
    one value, written as an unbinned category is ("2020"), not "2,020 – 2,021".
  A year column over five years then reads 2020, 2021, 2022, 2023, 2024 under "Automatic".
  The builder's default stays "Automatic" and `chart-builder.tsx` is not edited. No rule
  reads a column's name. A width a person types (0.5) is honoured as typed. A year column
  that spans more than ten years is still cut in ranges ("2,000 – 2,005"): open question 5.
- **D6. The donut's legend (finding 3).** Cause: the pie's legend is the only category
  legend without `type: "scroll"` (`chart-option.ts:123`; the rose and the funnel have it,
  `:166`, `:176`), so twenty entries wrap upward from the bottom over the chart and past the
  canvas. Fix: the same paged legend as the rose and the funnel: one line, with the pager
  ECharts draws, so every entry can be reached and none is clipped.
- **D7. `whole` on the wire.** `bins: { <column>: { mode, width, min, max, whole? } }`.
  `whole` is `1` or `0` and present only for automatic and counted bins (the rule of D5);
  absent on a width bin, a histogram and an older server. The console reads it as true only
  when it is `1` or `true`.
- **D8. Rows tables (finding 4).** The shared table `RowsTable` gains a switch. With it, a
  NULL cell reads "(No value)" and an empty string "(Empty)", through `categoryLabel`
  (`src/lib/chart-transforms.ts:23`, the helper the charts use; rule 4), in muted text so it
  is not taken for a stored value. It is switched on in the two tables that show a chart's
  own result: "Rows (N)" under the builder's preview and the "numbers behind this chart"
  dialog of a tile. It applies to every column of those tables (a NULL measure is "(No
  value)" too). The record list and the assistant's result card stay as they are. A CSV
  keeps the cell empty: a file holds values, not display labels.
- **D9. The records panel says the question (finding 5).** Only for a chart with more than
  one question. A line under the panel's title: "Question: <name>". The name is the
  question's legend name: the label of one of the other questions (`LayerCell.label`), or,
  for the chart's own question, the name of the clicked series. It is display only and is
  never sent to the server.
- **D10. A joined column's label (finding 6).** Decision 3 of
  `docs/core/features/joins-without-sql.md` says `table.column`. The label changes in the
  two places QA named: the record list (and its one-record view and the click menu's first
  line) through `displayColumnFull` (`src/lib/chart-joins.ts:114`), and the header of a
  downloaded raw table through a Rust twin of that function. A column label a person typed
  still wins. **The name in SQL and in stored definitions stays `<alias>__<column>`**: it is
  an identifier (`AS capacity__rooms`, `joins.rs:175`), it is the key of `columnSettings`,
  filters and measures in saved charts, and a dot inside an identifier would read as a
  table qualifier. Charts, pickers and table tiles keep the short name they show today
  (`displayColumn`, phase 2 QA R14).
- **D11. Excel and the apostrophe (finding 7).** Read in `rust_xlsxwriter` 0.102.0 (the
  pinned version, `lakehouse-api/Cargo.toml:204`; source in the cargo registry,
  `src/worksheet.rs`): `write_string` (`:2939-2947`) stores a string cell
  (`store_string`, `:15491`), which is written as `t="inlineStr"` (`:19679-19707`) or
  `t="s"` (`:19656-19674`). A formula is written only by `write_formula`, which the export
  never calls. A spreadsheet does not evaluate a string cell, so the apostrophe adds no
  protection in `.xlsx` and shows as data. Fix: an Excel text cell and an Excel header hold
  the text as it is. **CSV and TSV keep the apostrophe**: a text file has no cell types, the
  apostrophe is the accepted cost there, and `csv_export::neutralise` is not changed. The
  protection is pinned by a test: for `=`, `+`, `-` and `@` the workbook holds no formula
  element and a string cell. One thing is not protected and is said on the feature page: a
  reader who saves the workbook as CSV with their own program gets an unguarded text file
  (open question 1).
- **D12. Compact with a currency, and "IDR" (finding 8).** Read at `7ac20f8be`:
  - The compact amount already exists: `{ style: "compact", currency }` is formatted by
    `compactCurrency` (`value-format.ts:101-115`), stored by the server (`style.rs:111-113`,
    `:534-548`) and written to Excel (`export_format.rs`). What is missing is a way to find
    it: the style list offers "Currency" and "Compact (1.2 K)", and nothing says that
    Compact takes a currency code. Fix: one more entry in the style list, "Currency, compact
    (1.2 M)", which stores `{ style: "compact", currency }`. Nothing new is stored.
  - `Intl.NumberFormat` with `notation: "compact"` is **not** used: decision 3 of
    `docs/core/features/chart-formatting.md` says K, M, B, T in every locale, and `Intl`
    writes the locale's own words ("Rp 3,3 jt" in `id-ID`).
  - The symbol. The locale is the deployment's number locale (Admin > Settings; `en-US` by
    default, or `id-ID`; `src/lib/formatting-settings.ts:8-13`), which every tile reads as
    `reporting.numberLocale`. With `currencyDisplay: "symbol"`, `Intl` has no symbol for IDR
    in `en-US` and writes the code. Fix: for IDR only, the console asks `Intl` for
    `narrowSymbol`, which is "Rp" in both locales. Every other currency keeps `symbol`,
    because `narrowSymbol` writes "$" for SGD, CAD and AUD, which reads as US dollars.
  - What it prints (decimals set, so the text does not depend on the `Intl` version):

    | Format | `en-US` | `id-ID` |
    | --- | --- | --- |
    | currency, IDR, decimals 0, 1234 | `Rp 1,234` (today `IDR 1,234`) | `Rp 1.234` (unchanged) |
    | compact, IDR, decimals 2, 1234567.891 | `Rp 1.23 M` (today `IDR 1.23 M`) | `Rp 1,23 M` (unchanged) |
    | currency, IDR, display code, decimals 0 | `IDR 1,234` (unchanged) | `IDR 1.234` (unchanged) |
    | currency, USD, decimals 2, 1234.5 | `$1,234.50` (unchanged) | `US$1.234,50` (unchanged) |
    | currency, SGD, decimals 2, 1234.5 | `SGD 1,234.50` (unchanged) | `SGD 1.234,50` (unchanged) |

    The "today" and "unchanged" texts are in the repository's tests and case file; the new
    `en-US` texts were read from `Intl` in Node 22 on the planner's machine, not from `bun`.
    If `bun` prints something else for a row, stop and report; do not change the expected
    text. Formats without `decimals` are not asserted: the default digits of IDR differ
    between `Intl` versions.
  - The Rust twin (`export_format::symbol`) and its recorded cases change in the same task,
    so a CSV "formatted" file and an Excel number format say "Rp" too.

### BI-53: the PDF

- **D13. How the PDF is made (read, not assumed).**
  - Who renders: the `gotenberg` compose service (`gotenberg/gotenberg:8.37.0`, Chromium;
    `docker-compose.yml:1174-1183`), asked by the API: `pdf::convert_url`
    (`lakehouse-api/src/pdf.rs:244-292`), called by `make_pdf`
    (`routes/pdf_export.rs:195-216`) for the download route
    `GET /api/dashboard/export/pdf` and for scheduled reports. At most two at once
    (`PDF_AT_ONCE`, `state.rs:300`); 60 seconds at most (`PDF_DEADLINE`, `pdf.rs:27`).
  - What it opens: `<CONSOLE_INTERNAL_URL>/render/dashboard/<render token>`
    (`pdf_export.rs:281-323`). No query parameter and no header of ours. The token (two
    minutes) names the dashboard, the person, the filter, parameter and grain values, the
    paper and the orientation.
  - What the API asks of the renderer (`pdf.rs:250-277`): the paper's size in inches (A4
    8.27 by 11.7, Letter 8.5 by 11), `landscape`, margins of 0.5 in (0.7 at the bottom),
    `printBackground`, `emulatedMediaType: print`, `waitForExpression:
    window.__lakehouseRenderReady === true`, `failOnConsoleExceptions`, a footer. **No
    window or viewport width is sent** (the chart-image route sends `width` 1200,
    `pdf.rs:296`, `:340`); the renderer's own default applies and is not in this repository.
  - The page (`src/features/dashboards/render-view.tsx`): posts the token once to
    `POST /api/render/data`, gets the whole dashboard with every tab as the person, and
    draws every tab one under the other through `DashboardGrid`. One page load, one data
    read, one print: nothing is done once per PDF page. It says "ready" when every chart
    has reported its first `finished` and two frames have passed, or after 20 seconds
    (`READY_SAFETY_MS`, `:43`, `:128-140`) when a chart never reports.
  - Where the theme comes from: the dashboard's saved `theme` in that payload, through
    `dashboardLook(data, false, …)` (`render-view.tsx:147`). The console's own theme is
    never used (`appDark` is `false`): `auto` and `light` print light, `dark` prints dark.
  - Layout: the tiles are stacked full width by the print stylesheet only
    (`src/app/globals.css:174-189`, inside `@media print`): `.dash-tile` is `position:
    static`, `width: 100%`, `height: 360px`. This is the rule the feature page records under
    "Owner decisions at the phase 4 QA (2026-10-23)": every tile full width, one per row.
- **D14. Which findings have a cause shown by reading.**

  | Finding | Status | Task |
  | --- | --- | --- |
  | A dark dashboard prints with white bands | **cause shown** | T11 |
  | Tiles stay at half width | **mechanism shown; its numbers not shown: investigation** | T7, then T12 |
  | Scatter and bubble points cut at the right, PDF only | **mechanism shown (the same one); its numbers not shown: investigation** | T7, then T12 |
  | About 45 seconds for seven pages | **cause not shown: investigation** | T6, T7; fix in S2 |

- **D15. White bands: the PDF is always light.** Cause: the print stylesheet paints the
  page white (`globals.css:171`), the page's margins belong to no element, and the dark
  theme is painted only by the print view's own box (`render-view.tsx:185`), which ends
  where its content ends. A dark box on a white page has white bands wherever the box is
  not. Fix: the PDF's print view always uses the light theme, for the page, the cards and
  the charts. The chart image (PNG) keeps the dashboard's theme: its box is the whole
  picture. The browser's own Print (the "Print…" item, `dashboard-page.tsx:761-766`) is not
  changed (section 4).
- **D16. Half width and cut points: one mechanism.** What the code shows:
  1. a chart is a canvas sized once, from its box, when it is created (`echart.tsx:56`),
     and again only when a `ResizeObserver` fires in a running page (`:83-84`);
  2. the tiles are stacked full width only under `@media print` (D13), so the box a chart
     is created in depends on whether print media is already in force and on the window's
     width, neither of which this repository sets;
  3. printing lays the page out again at the paper's printable width (A4 landscape: 10.7 in
     = 1027 px; A4 portrait: 7.27 in = 697 px), and a canvas is not drawn again for it;
  4. a card clips what is wider than itself (`tile-card.tsx:73`, `overflow-hidden`).
  So a canvas narrower than the printed card leaves the card part empty ("half width"),
  and a canvas wider than it is cut at the card's right edge. Nothing in the print path is
  different for a scatter or a bubble chart (`render-view.tsx:160` calls the same
  `TileBody`); their points sit at the far right, so the cut is seen there. An axis does not
  get another domain in a narrower chart, so "the axis stops at 500K" is the cut, not
  another scale. Not shown by reading: the window's width under Gotenberg 8.37.0 and the
  moment print media applies. T7 measures both.
- **D17. The print layout rule.** The print view lays itself out at the paper's printable
  width in every medium, before any chart is created: its box has a fixed width in pixels
  (697 for A4 portrait, 1027 for A4 landscape, 720 for Letter portrait, 960 for Letter
  landscape: the paper's width minus the two 0.5 in margins `pdf.rs` sends, at 96 px per
  inch, rounded down), and the stacking rules hold for the print view without
  `@media print`. The layout a chart is created in is then the layout that is printed,
  whatever window the renderer has. The page stylesheet also has `@page { margin: 10mm }`
  (`globals.css:172`); which of the two margins Chromium uses is not established here. The
  width above is the smaller of the two cases, and the box is centred, so in the other case
  there is a strip of at most 10 px on each side and nothing is cut.
- **D18. Time: measured first, nothing optimised in S1.** By reading, the pipeline has no
  cost per PDF page and three fixed waits: a slot (only when two PDFs are being made), two
  frames, and the 20-second safety wait (only when a chart never reports). That does not
  explain 45 seconds, so no cause is shown and no fix is planned. S1 adds the API's own
  stage lines (T6) and measures the renderer alone on the build machine (T7). The numbers go
  to `docs/plans/BI-53-RESULT.md`. The stages of the real pipeline need a running product
  stack with a dashboard; the developer does not start one (the repository has no gate that
  does, and the build machine is shared): that part of the result file is written as *not
  measured*, and the planner fills it from staging after the merge.

### Words on screen (English, as the console). None of these has been seen by the product owner.

- Style list entry: **"Currency, compact (1.2 M)"**.
- Records panel: **"Question: <name>"**.
- Cell placeholders: **"(No value)"**, **"(Empty)"** (the existing constants).

### Shared files with dashboards-p2

Read from `/tmp/qa/plans/2026-11-12-qa-round-dashboards-p2.md` (its tasks T1 to T15). Line
numbers are at `7ac20f8be`.

| File | This slice | dashboards-p2 | Risk |
| --- | --- | --- | --- |
| `rust/crates/lakehouse-bi/src/store.rs` | T1: three assertions, `:6315`, `:7968`, `:8546` | its T1: two new tests after `:7984` | **By meaning, not by text**: its new test asserts `round(avg(stay)) AS avg_stay`, which T1 here turns into `round(avg(stay), 6) AS avg_stay`. Whichever slice merges second changes that one string |
| `rust/crates/lakehouse-api/src/routes/support.rs` | T2: `add_bins`, `:1039-1047` | its T4: a new function after `:995` | Different functions, 45 lines apart |
| `rust/crates/lakehouse-api/src/routes/dashboard.rs` | T5: `export_columns`, `:3803-3826` | its T4, T5: `:49-100`, `:138-148`, `:244-247`, `:446-481`, `:552-554`, `:629-630`, `:787-789`, `:1109`, `:2936`, `:4144`, tests in `mod time_grain` | Different regions |
| `src/features/dashboards/chart-option.ts` | T8: the pie's legend, `:123` | its T8: the radar branch, `:195-205`; it names `:123` as its model | Different branches. Its radar legend already asks for `type: "scroll"`; after T8 here the pie has it too |
| `src/features/dashboards/chart-option.test.ts` | T8: one test, inserted after the test at `:208` | its T8: two tests | Insert where the task says, not at the end |
| `src/features/dashboards/tile-dialogs.tsx` | T9: `TileDataDialog`, `:96` | its T9: the `Rows` type, `:33-34` | 60 lines apart |
| `src/features/dashboards/tile-body.tsx` | T10: the click of a combined chart, `:412-421` | its T10: the import at `:12`, a line after `:434` | About 12 lines apart: the nearest pair |
| `src/features/dashboards/combined-tile.test.tsx` | T10: one test, inserted after the test at `:58-67` | its T10: one test | Insert where the task says, not at the end |
| `src/features/dashboards/dashboard-page.tsx` | T10: `:1021-1026`, `:1481` | its T9, T11: `:104-135`, `:222-223`, `:305-356`, `:453`, `:472-509`, `:1156`, `:1317-1329` | Different regions |
| `docs/core/features/chart-builder-questions.md` | T13: the **first** bullet of "Limits to tell a customer" | its T15: a new last row of Decisions, a new last line of Limits | Insert at the top of the list, as the task says |

Not shared: this slice edits `rust/crates/lakehouse-bi/src/builder.rs` and dashboards-p2
does not; neither slice edits `src/features/dashboards/chart-builder.tsx`
(dashboards-p2 adds a test to `chart-builder.test.tsx`, this slice does not); dashboards-p2
edits `src/lib/chart-shape.ts`, `combined-option.ts`, `page-context-summary.ts`,
`grain-switch.ts`, `params.ts` and this slice does not.

## 2. Tasks

Rust first (T1 to T7; T2, T4 and T5 carry their console half in the same commit, because
one side without the other is a wrong label), then the console (T8 to T10), the PDF (T11,
T12), the documents (T13).

### T1. An average, a minimum and a maximum keep six decimals (LH-BI-6-04; BI-55, D4)

- Anchors: `rust/crates/lakehouse-bi/src/specs.rs:159-171` (`enum Aggregate`), `:173-205`
  (its `impl`), `:207-211` (`Display`), `:483` (the tests);
  `rust/crates/lakehouse-bi/src/builder.rs:979` (a doc comment that quotes
  `round(avg(v))`), `:1047-1060` (`agg_expr`), `:1130-1139` (`kpi_value`), `:1436-1440`
  (`build_points_sql`); `rust/crates/lakehouse-bi/src/bin.rs:595-599` (`grid_points_sql`);
  `rust/crates/lakehouse-bi/src/tables.rs:939-945`;
  `rust/crates/lakehouse-bi/src/builder/steps.rs:204-208`;
  `rust/crates/lakehouse-api/tests/breakdown_cap.rs` (whole file: the model of a test on a
  real ClickHouse).
- Change:
  1. `specs.rs`, in `impl Aggregate`: `pub const DECIMAL_SCALE: u8 = 6;` and
     `#[must_use] pub fn rounded(self, operand: &str) -> String`:
     `Count` → `"count()"`; `Sum` → `format!("round(sum({operand}))")`;
     `Avg`, `Max`, `Min` → `format!("round({self}({operand}), {})", Self::DECIMAL_SCALE)`.
     The doc comment says what D4 says (why six, why a sum stays whole, that `round(x, 6)`
     of a whole number is the number) and names the QA case.
  2. Replace the six hand-written `round(…)` by a call, keeping every surrounding branch
     (calculated field, `count`) as it is:
     - `builder.rs` `agg_expr`: the last `else` returns
       `self.measure_aggregate(measure).rounded(measure.column.as_str())`;
     - `builder.rs` `kpi_value`: `agg.rounded(measure.as_str())`;
     - `builder.rs` `build_points_sql`: `format!("{} AS {measure}", agg.rounded(measure.as_str()))`;
     - `bin.rs:598`: the same form as the line above;
     - `tables.rs:944`: `agg.rounded(measure.as_str())`;
     - `steps.rs:207`: `format!("{} AS __s{i}", Aggregate::from_str_lossy(&agg).rounded(&m.column))`
       (`agg` was checked against the five words three lines above; import
       `crate::specs::Aggregate` if the file does not have it).
     If an argument is not a `&str` at a site, convert it there (`.as_str()` or
     `&x.to_string()`); do not change the helper's signature.
  3. `builder.rs:979`: in the doc comment write `round(avg(v), 6) AS avg_v`.
  4. The pinned statements. Each of these assertions changes because the rule changed
     (rule 2: say so in the commit body); change the quoted fragment and nothing else:
     - `builder.rs:5191-5192` (`two_aggregations_of_one_column_are_two_result_columns`):
       `round(avg(stay), 6) AS avg_stay` and `round(max(stay), 6) AS max_stay`;
     - `builder.rs:5208`: `round(avg(visitors), 6) AS __m1`;
     - `builder.rs:5303`: `round(avg(stay), 6) AS avg_stay`;
     - `builder.rs:5414`: `round(avg(stay), 6) AS __m0`;
     - `builder.rs:5474`: `round(max(visitors), 6) AS __s0`;
     - `store.rs:6315`: `round(avg(visitors), 6) AS visitors`;
     - `store.rs:7968`: `round(avg(stay), 6) AS avg_stay`;
     - `store.rs:8546`: `round(avg(capacity__rooms), 6) AS capacity__rooms`.
     Afterwards `grep -rn 'round(avg(\|round(max(\|round(min(' rust/crates src ops` shows
     only the six-decimal form. No `round(sum(` line changes anywhere.
- Check:
  - `specs.rs` tests,
    **`a_sum_is_a_whole_number_and_an_average_a_minimum_and_a_maximum_keep_six_decimals`**:
    `Aggregate::Sum.rounded("v")` is `round(sum(v))`; `Avg` → `round(avg(v), 6)`; `Max` →
    `round(max(v), 6)`; `Min` → `round(min(v), 6)`; `Count` → `count()`.
  - New file `rust/crates/lakehouse-api/tests/chart_numbers.rs`, modelled on
    `breakdown_cap.rs` (one ClickHouse `26.8` container per binary through a `OnceCell`, the
    same three environment variables, no `mod common`). Module doc: `BI-55`, why the engine
    is the only place these rules are decided. Seed
    `default.bi55_avg (kab String, v UInt32)` with `('a', 1), ('a', 2), ('a', 2), ('b', 10)`.
    A helper `fn number(v: &Value) -> f64` reads a JSON number or a quoted number (the
    engine quotes 64-bit integers).
    **`an_average_keeps_its_decimals_and_a_maximum_of_whole_numbers_stays_whole_on_the_engine`**:
    build the chart through `spec_from_inline_sql` as the model does, source
    `SELECT kab, v FROM default.bi55_avg`, columns `kab` `String` and `v` `UInt32`, input
    `{ "title": "BI-55 avg", "kind": "bar", "dimension": "kab", "measures": ["v", {"column": "v", "aggregate": "avg"}, {"column": "v", "aggregate": "max"}] }`.
    The saved statement contains `round(avg(v), 6)` and `round(max(v), 6)`. Run it. In the
    row whose `kab` is `a`: `number(avg_v)` is `1.666667`, `number(max_v)` is `2.0`,
    `number(v)` is `5.0`; in the row `b`: `10.0`, `10.0`, `10.0`.
    If the engine refuses the statement or answers another number, stop and report the
    statement and the answer; do not change the expected value (rule 2).
  The unit test fails on today's code (the function does not exist); the engine test fails
  on it too (the average is `2`).

### T2. Bins of a whole-number column are never narrower than 1 and read as their value (LH-BI-2-02; BI-55, D5, D7)

- Anchors: `rust/crates/lakehouse-bi/src/bin.rs:36-43` (the module doc, "How the width is
  chosen"), `:356-397` (`stats_sql`; the `Nice` branch is `:383-396`), `:413-420`
  (`read_sql`), `:669-687` (two tests that must pass unchanged);
  `rust/crates/lakehouse-api/src/routes/support.rs:1020-1065` (`add_bins`; the object is
  built at `:1039-1047`); `rust/crates/lakehouse-api/src/routes/bin_rewrite.rs:304`
  (`every_bin_mode_reads_the_table_only_through_its_governed_projection`, must pass
  unchanged); `src/lib/chart-bins.ts:111` (`BinMeta`), `:114` (`BinInfo`), `:126-149`
  (`binsOfTile`), `:152-156` (`binLabel`), `:165-178` (`relabelBins`);
  `src/lib/chart-bins.test.ts:35-57`.
- Change, Rust:
  1. `bin.rs`, `stats_sql`, **the `Nice` branch only** (the `Fixed`, `Exact` and `Nearest`
     statements stay byte for byte). With `c` the column and `base` the relation:
     - its own first level:
       `SELECT min(toFloat64({c})) AS mn, max(toFloat64({c})) AS mx, max(toFloat64({c}) != floor(toFloat64({c}))) AS fr FROM {base}`
       (`fr` is 1 when any value has a fraction);
     - the second level selects `mn, mx, fr, if(mx > mn, pow(10, floor(log10((mx - mn) / {n}))), 1) AS p`;
     - the last level is
       `SELECT mn, mx, fr, greatest(multiIf(<the branches as today>), if(fr = 0, 1, 0)) AS w FROM (<second level>)`.
     Comment at the site: QA `LH-BI-2-02` (`BI-55`), a column of whole numbers is never cut
     narrower than 1; a NULL `fr` (no rows) leaves the width as it was.
  2. `read_sql`: for a `WidthRule::Nice` range the select list is
     `w AS width, mn AS min, mx AS max, fr = 0 AS whole`; for every other rule it is what it
     is today. `scalars_sql` is not changed. Update both doc comments.
  3. The module doc: add two sentences to "How the width is chosen" saying the rule and
     that the tile carries `whole`.
  4. `support.rs`, `add_bins`: build the entry as today, then, only when
     `row["whole"]` is not null, insert `"whole": row["whole"]`. Update the doc comment of
     `annotate_bins` (`:997-1003`): `{ mode, width, min, max, whole? }`. A tile whose read
     has no `whole` (a width bin, a histogram, a mock) gets no key, so no existing
     assertion on `bins` changes.
- Change, console (`src/lib/chart-bins.ts`):
  1. `BinMeta` gains `whole?: unknown`; `BinInfo` gains `whole?: boolean` (comment: every
     value of the column is a whole number, said by the server for automatic and counted
     bins, `BI-55`).
  2. `binsOfTile`: when `m.whole` is `true` or `finite(m.whole) === 1`, the entry also has
     `whole: true`. Otherwise the entry has no such key (the existing `deepEqual`
     assertions must pass unchanged).
  3. `binLabel(lower, width, whole = false)`: first line, when
     `whole && width === 1 && Number.isInteger(lower)`, return `String(lower)`. The rest is
     unchanged. Update the doc comment.
  4. `relabelBins` passes `bins[column].whole` as the third argument.
  No other file of the console changes: `tile-body.tsx` reads these through `binsOfTile`
  and `relabelBins`, and the histogram and the page context call `binLabel` with two
  arguments.
- Check:
  - `bin.rs` tests,
    **`a_column_of_whole_numbers_is_never_cut_narrower_than_one_and_the_tile_is_told`**:
    with `Range::new(&col("tahun"), BinMode::Auto)`: `read_sql("serving.t")` starts with
    `SELECT w AS width, mn AS min, mx AS max, fr = 0 AS whole FROM (`, contains
    `max(toFloat64(tahun) != floor(toFloat64(tahun))) AS fr FROM serving.t`, contains
    `greatest(multiIf(ceil(mx / (1 * p)) - floor(mn / (1 * p)) <= 10, 1 * p` and
    `1000 * p), if(fr = 0, 1, 0)) AS w`, and names `serving.t` once;
    `scalars_sql("serving.t")` starts with
    `SELECT w AS __bw_tahun, mn AS __bmn_tahun, mx AS __bmx_tahun FROM (` and does not
    contain `whole`; `Range::new(&col("tahun"), BinMode::Width(500.0)).read_sql("serving.t")`
    does not contain `whole` or ` fr`.
  - `tests/chart_numbers.rs` (the file of T1): seed
    `default.bi55_years (tahun UInt16, stay Float64)` with
    `(2020, 1.5), (2021, 2.0), (2022, 2.5), (2023, 3.0), (2024, 4.5)`.
    **`automatic_bins_of_a_year_column_are_one_year_wide_and_a_fractional_column_keeps_its_width`**:
    run `Range::new(&Ident::new("tahun")?, BinMode::Auto).read_sql("default.bi55_years")`:
    `number(width)` is `1.0`, `min` `2020.0`, `max` `2024.0`, `whole` `1.0`. The same for
    `stay`: `width` `0.5`, `whole` `0.0`. Then run
    `SELECT {bin} AS b, count() AS n FROM (SELECT * FROM default.bi55_years CROSS JOIN ({scalars}) AS rng) GROUP BY b ORDER BY b`
    with `bin` the range's `bin_expr()` and `scalars` its `scalars_sql("default.bi55_years")`
    for `tahun`: five rows, `b` 2020 to 2024, each `n` 1. If the engine refuses a statement
    or answers another number, stop and report; do not change the expected value.
  - `cargo test -p lakehouse-api --lib bin_rewrite` passes unchanged (the role rewrite
    still reads the table only through its projection).
  - `src/lib/chart-bins.test.ts`:
    **"a bin of width 1 over whole numbers reads as its one value, with no separator"**:
    `binLabel(2020, 1, true)` is `"2020"`; `binLabel(2020, 1)` is `"2,020 – 2,021"`;
    `binLabel(2020, 5, true)` is `"2,020 – 2,025"`; `binLabel(0.5, 1, true)` is
    `"0.5 – 1.5"`.
    **"the tile's bins say whole only when the server said so"**:
    `binsOfTile({ tahun: { mode: "auto", width: 1, min: 2020, max: 2024, whole: 1 } }, undefined)`
    is `{ tahun: { width: 1, max: 2024, whole: true } }`; with `whole: 0` and with no
    `whole` it is `{ tahun: { width: 1, max: 2024 } }`.
    **"a year in a bin of one reads as the year"**:
    `relabelBins([{ tahun: 2020, v: 1 }], { tahun: { width: 1, whole: true } })` is
    `[{ tahun: "2020", v: 1 }]`.
  All new tests fail on today's code. No test drives `add_bins` (it needs the engine, the
  gate and the policy store together, and none exists today): say so in the handoff.

### T3. An Excel text cell holds its text without the apostrophe (LH-BI-1-08; BI-55, D11)

- Anchors: `rust/crates/lakehouse-api/src/csv_export.rs:21-29` (`cell_text`), `:31-40`
  (`neutralise` and its doc); `rust/crates/lakehouse-api/src/export.rs:22-25` (the module
  doc's sentence on neutralising), `:522-528` (`xlsx_cell_bytes`), `:565-570` (the header
  cells), `:575-579` (the header's bytes), `:617-625` (a text cell), `:761-773` and
  `:919-953` (tests); `rust/crates/lakehouse-api/tests/query_download.rs:485-527`
  (`a_cell_that_starts_a_formula_is_guarded_in_every_text_format`), `:566-578`
  (`workbook_text`).
- Change:
  1. `csv_export.rs`: `pub(crate) fn plain_text(value: Option<&Value>) -> String`: the
     match of `cell_text` without `neutralise` (null → empty, a number and a boolean as
     their text, a string as it is, anything else its JSON text). `cell_text` becomes: a
     number, a boolean and a null → `plain_text(value)`; everything else →
     `neutralise(&plain_text(value))`, so the match exists once. `neutralise` itself is
     **not** changed. Its doc comment: the comma file and the tab-separated file call it;
     an `.xlsx` text cell does not, because it is a typed string cell that a spreadsheet
     never evaluates (QA `LH-BI-1-08`, `BI-55`).
  2. `export.rs`, in `write_xlsx_file` and `xlsx_cell_bytes`, four sites: the header is
     written with the label as it is (`label.as_str()`), its bytes are `label.len()`, a
     text cell's text is `csv_export::plain_text(Some(other))`, and `xlsx_cell_bytes`
     counts `csv_export::plain_text(Some(value)).len()`. Keep `write_string` and
     `write_string_with_format`: never `write_formula`, never `write` (which guesses a
     type). Put the comment naming the case at the text-cell site.
  3. `export.rs` module doc `:22-25`: say the rule as D11 does.
  CSV and TSV code paths are not touched.
- Check:
  - `export.rs` tests,
    **`a_text_that_starts_like_a_formula_is_a_string_cell_in_the_workbook_with_no_apostrophe_and_no_formula`**:
    five rows through `source(5, make)` with a label for `name` of `=Name`; `make(i)`
    returns the `i`-th of `=1+1`, `+SUM(1,2)`, `-2+3`, `@SUM(1)`,
    `=HYPERLINK("http://example.invalid","x")` and a number. Write `FileFormat::Xlsx`, open
    `xl/worksheets/sheet1.xml` and, if present, `xl/sharedStrings.xml` as the test at
    `:919` does. The sheet contains neither `<f>` nor `<f `; it contains `t="inlineStr"` or
    `t="s"`; the two texts together contain `=1+1`, `+SUM(1,2)`, `-2+3`, `@SUM(1)`,
    `=HYPERLINK(` and `=Name`; and contain none of `'=1+1`, `'+SUM`, `'-2+3`, `'@SUM`,
    `'=HYPERLINK`, `'=Name`.
  - `export.rs:941-952`: the last assertion of
    `xlsx_keeps_numbers_as_numbers_with_the_charts_number_format` becomes: the texts
    contain `=HYPERLINK` and do not contain `'=HYPERLINK`; correct the comment above it.
  - `tests/query_download.rs:521-526`: the workbook's text contains `=1+1`, and contains
    none of `'=1+1`, `&apos;=1+1`, `<f>`, `<f `. The CSV and TSV halves of the test stay
    as they are. Correct the test's doc comment (`:485-487`).
  - These must pass **unchanged**:
    `csv_export::tests::a_text_that_could_run_as_a_formula_is_prefixed_and_a_number_is_not`,
    `csv_export::tests::a_tab_separated_file_guards_a_formula_the_way_the_comma_file_does`,
    `export::tests::a_quoted_64_bit_integer_is_a_number_and_a_formula_is_neutralised`.
  The two changed assertions change because the rule changed, and the new ones also
  forbid a formula element (rule 2: say so in the commit body). The new test fails on
  today's code.

### T4. A rupiah amount reads "Rp" in both locales, and a compact amount can be chosen (LH-BI-1-06; BI-55, D12)

- Anchors: `rust/crates/lakehouse-bi/src/export_format.rs:75-96` (`symbol` and its doc),
  `:429-445` (`idr_compact_two_decimals_is_a_three_tier_excel_format`);
  `rust/crates/lakehouse-bi/src/export_format_cases.jsonl` (340 lines);
  `rust/crates/lakehouse-api/tests/dashboard_export.rs:726`, `:780`;
  `src/lib/value-format.ts:83-95` (the `currency` case), `:101-115` (`compactCurrency`);
  `src/lib/value-format.test.ts`; `src/features/dashboards/format-section.tsx:25-27`
  (`STYLE_LABELS`), `:52-86` (`ValueFormatEditor`; the style list is `:63-66`);
  `src/features/dashboards/format-section.test.tsx:1-20` (`choose`), `:43-50`.
- Before any edit, run this from the repository root and keep its output for the handoff
  (it compares the console's formatter with the recorded cases; it is not committed):

  ```bash
  bun -e '
  import { readFileSync } from "node:fs"
  import { formatValue } from "./src/lib/value-format"
  let bad = 0
  for (const line of readFileSync("rust/crates/lakehouse-bi/src/export_format_cases.jsonl", "utf8").split("\n")) {
    if (!line.trim()) continue
    const [locale, fmt, value, text] = JSON.parse(line)
    if ((fmt.style ?? "auto") === "auto") continue
    const got = formatValue(value, fmt, locale)
    if (got !== text) { bad += 1; console.log(JSON.stringify({ line, got })) }
  }
  console.log(`${bad} lines differ`)
  '
  ```

  Expected: `0 lines differ`. If lines differ before any edit, list them in the handoff
  and leave them alone: they are not this task's.
- Change:
  1. `value-format.ts`: a private function next to `plainSpaces`,
     `currencyDisplayOf(fmt: ValueFormat): "symbol" | "narrowSymbol" | "code"`: the format's
     display (`symbol` when absent), except that `symbol` for the code `IDR` (also when the
     code is absent, which reads as IDR) is `narrowSymbol`. Use it at both `Intl` calls
     (`:85-90`, `:109`). Comment: QA `LH-BI-1-06` (`BI-55`) and the reason of D12 (the
     English locale has no symbol for IDR; other currencies keep `symbol` because the narrow
     symbol of SGD, CAD and AUD is "$").
  2. `export_format.rs`, `symbol`: the arm `("IDR", true)` becomes `("IDR", _)`. Rewrite the
     doc comment's sentence about `IDR 1,234.00` in `en-US`.
  3. `export_format_cases.jsonl`: in the 40 lines whose first element is `"en-US"`, whose
     format has `"currency":"IDR"` and has no `"currencyDisplay":"code"`, the fourth
     element's leading `IDR ` becomes `Rp ` and a leading `-IDR ` becomes `-Rp `. Nothing
     else in the file changes. Run the command above again: `0 lines differ` (or exactly the
     lines listed before the edit).
  4. `format-section.tsx`: a local list
     `STYLE_CHOICES = ["auto", "number", "percent", "currency", "currencyCompact", "scientific", "compact"]`
     and one more label, `currencyCompact: "Currency, compact (1.2 M)"`. In
     `ValueFormatEditor`:
     - the list shows `currencyCompact` when the stored style is `compact` and a currency
       is stored, else the stored style;
     - choosing `currencyCompact` stores `{ ...f, style: "compact", currency: f.currency ?? "IDR" }`;
     - choosing `compact` stores `{ ...f, style: "compact", currency: undefined, currencyDisplay: undefined }`;
     - any other choice stores the style as today.
     The currency inputs stay shown for `currency` and `compact` as today.
     `FORMAT_STYLES` in `src/lib/chart-style.ts` is not changed (it mirrors the server).
- Check:
  - `value-format.test.ts`:
    **"a rupiah amount reads Rp in the English number locale too"**:
    `formatValue(1234567.891, { style: "compact", currency: "IDR", decimals: 2 }, "en-US")`
    is `"Rp 1.23 M"`; the same value negated is `"-Rp 1.23 M"`;
    `formatValue(1234, { style: "currency", currency: "IDR", decimals: 0 }, "en-US")` is
    `"Rp 1,234"`.
    **"a rupiah amount shown as its code still reads IDR, and other currencies keep their symbol"**:
    with `currencyDisplay: "code"` and decimals 0, `en-US`: `"IDR 1,234"`;
    `formatValue(1234.5, { style: "currency", currency: "USD", decimals: 2 }, "en-US")` is
    `"$1,234.50"` and in `id-ID` `"US$1.234,50"`; with `currency: "SGD"` in `en-US`
    `"SGD 1,234.50"`.
  - `export_format.rs`: in `idr_compact_two_decimals_is_a_three_tier_excel_format` the
    `en-US` half expects
    `[>=1000000000]"Rp "#,##0.00,,," B";[>=1000000]"Rp "#,##0.00,," M";"Rp "#,##0.00`
    (the comment above it says why). `the_csv_text_is_what_the_console_shows_for_every_recorded_case`
    passes with the edited file. The case with `currencyDisplay: "code"` stays `"IDR "#,##0`.
  - `tests/dashboard_export.rs:726`: `styles.contains("Rp ")`; `:780`: the CSV is
    `"\u{feff}email,amount\r\n***,Rp 1.23 M\r\n"`.
  - `format-section.test.tsx`, in `describe("ValueFormatEditor")`:
    **"offers a compact amount of money as one choice and stores it as the compact style with the currency"**:
    with `{ style: "currency", currency: "USD" }`, `choose(<the "Format style" trigger>, "Currency, compact (1.2 M)")`
    calls `onChange` with `{ style: "compact", currency: "USD" }`; with `{ style: "number" }`
    it is `{ style: "compact", currency: "IDR" }`.
    **"shows a stored compact amount as that choice, and plain Compact drops the currency"**:
    with `{ style: "compact", currency: "IDR", currencyDisplay: "code" }` the trigger's text
    contains `Currency, compact`; choosing "Compact (1.2 K)" calls `onChange` with a format
    whose `style` is `compact` and whose `currency` and `currencyDisplay` are undefined.
    The existing test "shows the currency controls only for the currency and compact
    styles" passes unchanged.
  The changed Rust assertions change because the rule changed (rule 2). The new tests fail
  on today's code.

### T5. A joined column reads `table.column` in the record list and in a downloaded header (LH-BI-7-04; BI-55, D10)

- Anchors: `rust/crates/lakehouse-bi/src/joins.rs:632-645` (the last function before the
  tests); `src/lib/chart-joins.ts:84-117` (`splitJoined`, `displayColumn`,
  `displayColumnFull`: the rule to mirror);
  `rust/crates/lakehouse-api/src/routes/dashboard.rs:3803-3826` (`export_columns`);
  `rust/crates/lakehouse-api/tests/dashboard_export.rs:466-470`;
  `src/features/dashboards/drill.tsx:59-61` (the menu's first line), `:161` (the list),
  `:211-216` (the one-record view); `src/features/dashboards/drill.test.tsx:32-52` (the
  model).
- Change:
  1. `joins.rs`, before the tests: `#[must_use] pub fn display_name(name: &str) -> String`.
     A name that starts with `__` is returned as it is; otherwise it is split at the first
     `__`, and when both sides are not empty the answer is `<left>.<right>`; any other name
     is returned as it is. Doc comment: the label of a flat joined column in a file's
     header (feature decision 3), the twin of `displayColumnFull` in
     `src/lib/chart-joins.ts`; the name in SQL stays `<alias>__<column>`, for the reason of
     D10; QA `LH-BI-7-04` (`BI-55`).
  2. `dashboard.rs`, `export_columns`: a column with no label of its own takes
     `lakehouse_bi::joins::display_name(c)` when `chart.def.shape.joins_list()` is not
     empty, and its name as today when the chart has no join.
  3. `drill.tsx`: import `displayColumnFull` from `@/lib/chart-joins`. The list gets
     `labels={Object.fromEntries(page.columns.map((c) => [c, displayColumnFull(c)]))}`; the
     one-record view writes `displayColumnFull(c)` in its `dt`; the menu's first line
     writes `displayColumnFull(drill.column)`.
- Check:
  - `joins.rs` tests,
    **`a_joined_column_reads_table_dot_column_and_any_other_name_is_left_alone`**:
    `mart_y__email` → `mart_y.email`; `capacity2__rooms` → `capacity2.rooms`; `email` →
    `email`; `__other` → `__other`; `x__` → `x__`; `a___b` → `a._b`.
  - `tests/dashboard_export.rs:468`: the header starts with
    `email,mart_y.email,mart_y.secret`.
  - `drill.test.tsx`, **"a joined column is headed table.column in the list and in the one-record view"**:
    a page whose columns are `["kota", "zz_hotel__kamar"]`: the header `zz_hotel.kamar` is
    on the page and `zz_hotel__kamar` is not; after a click on the first row the field's
    name reads `zz_hotel.kamar`.
  The changed assertion in `dashboard_export.rs` changes because the rule changed (rule
  2). All three fail on today's code.

### T6. The API logs how long each stage of a PDF took (LH-BI-1-01 F3; BI-53, D18)

- Anchors: `rust/crates/lakehouse-api/src/routes/pdf_export.rs:195-216` (`make_pdf`),
  `:261-266` (`take_slot`); `rust/crates/lakehouse-api/src/routes/render.rs:69-150`
  (`data`; `payload_for` is called at `:131`).
- Change (numbers only: no token, no name, no address, no error text in any line):
  1. `make_pdf`: take `std::time::Instant::now()` before `take_slot`; after the slot,
     `tracing::info!(slot_wait_ms, "pdf render started")`; after `convert_url`,
     `tracing::info!(render_ms, ok, "pdf render finished")`, where `render_ms` counts from
     the slot and `ok` is whether the conversion succeeded. Return the result as today.
  2. `render::data`: after the token has verified and the person may read,
     `tracing::info!("render data requested")`; around `payload_for`,
     `tracing::info!(payload_ms, ok, "render data read")`.
  3. Add two lines to the module doc of `pdf_export.rs`: the four lines, in order, are the
     stages of one PDF (slot wait; renderer start and page load; the dashboard's read;
     draw, wait for ready and print), and they hold durations only.
- Check: no unit test (the change is four log lines; the crate has no log capture in its
  tests). `cargo clippy -p lakehouse-api --all-targets -- -D warnings`,
  `cargo test -p lakehouse-api --test sec11_guard`, `--test route_auth`, and
  `cargo test -p lakehouse-api --lib pdf_export` (the existing tests of the file) pass.
  Say in the handoff that the lines are *not verified* on a running stack.

### T7. Probe the renderer, and write down what is measured and what is not (BI-53 investigation; D16, D18)

- Anchors: `rust/crates/lakehouse-api/tests/pdf_real_gotenberg.rs` (whole file: its module
  notes hold the container command; its first test is the model).
- Change, a third kind of test in that file,
  **`the_renderer_reports_its_window_its_media_and_how_long_a_conversion_takes`**
  (`#[tokio::test]`, `#[ignore = "needs a Gotenberg container, see the module notes"]`):
  1. The page it serves (one constant):

     ```html
     <!doctype html><html><body style="margin:0"><div id="box" style="width:100%;height:10px"></div><script>
     function report(stage) {
       var box = document.getElementById('box').getBoundingClientRect().width;
       return fetch('/probe?stage=' + stage + '&w=' + window.innerWidth + '&box=' + box + '&print=' + window.matchMedia('print').matches);
     }
     report('start').then(function () { setTimeout(function () { report('later').then(function () { window.__lakehouseRenderReady = true; }); }, 1500); });
     </script></body></html>
     ```

  2. The listener is the model's, with one difference: it keeps the first line of every
     request in an `Arc<Mutex<Vec<String>>>`; a request whose first line contains
     `/probe?` is answered `200` with an empty body, any other with the page.
  3. The test calls `convert_url` twice, in this order: `(Paper::A4, Orientation::Landscape)`
     then `(Paper::A4, Orientation::Portrait)`. After each it prints, with `println!`, the
     orientation, the elapsed milliseconds and the `/probe?` lines received since the last
     call. It asserts that each call returned a PDF and that two `/probe?` lines arrived
     for each. It asserts nothing about the numbers: reading them is the purpose. Its doc
     comment says so and names `BI-53`.
- Procedure (in the foreground; the container is removed by `docker stop`):

  ```bash
  docker run -d --rm --name bi53-gotenberg-scratch -p 127.0.0.1:13353:3000 \
    --add-host=host.docker.internal:host-gateway gotenberg/gotenberg:8.37.0 \
    gotenberg --api-timeout=70s --log-level=warn
  GOTENBERG_TEST_URL=http://127.0.0.1:13353 cargo test -p lakehouse-api \
    --test pdf_real_gotenberg -- --ignored --nocapture --test-threads=1 \
    the_renderer_reports_its_window_its_media_and_how_long_a_conversion_takes
  docker stop bi53-gotenberg-scratch
  ```

  Run it on a container that has just started, so the first conversion includes the
  browser's own start. If the image has no build for the machine or the container cannot
  start, write *not verified locally* with the first line of the error; do not install
  anything.
- Write `docs/plans/BI-53-RESULT.md` (new), with these sections and nothing invented:
  1. **What was measured**: the machine (architecture, CPU count), the image tag, the date,
     the command, and a table copied from the test's output: orientation, elapsed ms, and
     for `start` and `later` the values `w`, `box`, `print`.
  2. **What the numbers say about the layout**, one row of this table chosen for each
     orientation, with no other verdict:

     | Seen | Meaning |
     | --- | --- |
     | `print` is `false` at `start` | A chart created early is created in the screen layout: half-width tiles at their grid width. This is finding F1 as QA saw it |
     | `box` at `later` is smaller than the paper's printable width (1027 landscape, 697 portrait) | Charts are narrower than the printed card: the card is part empty |
     | `box` at `later` is larger than the paper's printable width | Charts are wider than the printed card and are cut at its right edge: `LH-BI-6-22`, `LH-BI-6-23` |
     | `print` is `true` at `start` and `box` equals the printable width in both orientations | **The plan's reading is wrong.** Say so, and see T12 |

  3. **What the numbers say about the time**: the two elapsed times. The first conversion
     minus the second is the browser's start; the second is the renderer's own time for an
     empty page. Each includes the probe's own 1.5 seconds of waiting. No verdict on the 45
     seconds.
  4. **Not measured**: the stages of a real dashboard's PDF (slot wait; renderer start and
     page load; the dashboard's read; draw, wait for ready and print), each written as
     *not measured*, with the reason of D18 and the way to measure it: download one PDF of
     the dashboard of `LH-BI-1-01` on a deployment that runs this branch and read the four
     lines of T6 in the API's log; the gaps between them are the stages. And this table,
     for whoever reads those numbers (S2):

     | Seen | Points at |
     | --- | --- |
     | `slot_wait_ms` large | two PDFs were already being made |
     | long gap from "pdf render started" to "render data requested" | the renderer's start or the console page's load |
     | `payload_ms` large | the dashboard's read as that person |
     | about 20 seconds or more from "render data read" to "pdf render finished" | the safety wait: a chart was created and never reported that it finished |

- Check: the test compiles and is ignored by default
  (`cargo test -p lakehouse-api --test pdf_real_gotenberg` reports it ignored); the
  procedure's output is quoted in the handoff and in the result file.

### T8. A donut's legend pages instead of running over the chart (LH-BI-6-39; BI-55, D6)

- Anchors: `src/features/dashboards/chart-option.ts:123` (the pie's legend), `:166` (the
  rose's), `:176` (the funnel's); `src/features/dashboards/chart-option.test.ts:208-211`.
- Change: at `:123` add `type: "scroll"` to the legend, so it is the rose's legend key for
  key. Nothing else on the pie changes. Comment naming the case.
- Check, `chart-option.test.ts`, **inserted directly after the test at `:208`** (not at the
  end of the file: dashboards-p2 adds tests there),
  **"a donut's legend pages when its entries do not fit, like a rose's and a funnel's"**:
  for `kind` `pie`, `rose` and `funnel` over 25 rows, `buildOption(...).legend` has
  `type: "scroll"`. It fails on today's code for `pie`.

### T9. The Rows tables name a missing and an empty value as the chart does (LH-BI-6-38; BI-55, D8)

- Anchors: `src/components/patterns/rows-table.tsx:11-28` (the props), `:72-76` (a cell);
  `src/lib/chart-transforms.ts:21-25` (`categoryLabel`);
  `src/features/dashboards/chart-shape-section.tsx:427-449` (`PreviewRowsTable`; the table
  is at `:445`); `src/features/dashboards/tile-dialogs.tsx:96`.
- Change:
  1. `rows-table.tsx`: a prop `namePlaceholders?: boolean`, documented: a NULL cell reads
     "(No value)" and an empty string "(Empty)", in muted text, as a chart names them;
     absent, both are blank as before. In the cell: when the prop is set and the value is
     `null`, `undefined` or `""`, the text is `categoryLabel(value)` and the cell's class
     gains `text-muted-foreground`; otherwise the cell is what it is today. Import
     `categoryLabel` from `@/lib/chart-transforms`.
  2. `chart-shape-section.tsx:445` and `tile-dialogs.tsx:96`: pass `namePlaceholders`.
     `downloadRowsCsv` is not changed. The record list (`drill.tsx`) and
     `agent-result-card.tsx` do not pass it.
- Check:
  - New `src/components/patterns/rows-table.test.tsx`:
    **"names a missing value (No value) and an empty one (Empty) when asked"** (rows
    `[{ a: null, n: 1 }, { a: "", n: 2 }, { a: "Bali", n: 3 }]`; both labels are on the
    page, each in an element with the muted class) and
    **"leaves both blank when not asked"**.
  - `chart-shape-section.test.tsx`,
    **"the Rows table under the preview names a missing and an empty category"**: render
    `PreviewRowsTable` with `open` and those rows.
  - `tile-dialogs.test.tsx`,
    **"the numbers behind a chart name a missing and an empty category"**: render
    `TileDataDialog` with those rows.
  The first, third and fourth fail on today's code.

### T10. The records panel says which question the records belong to (LH-BI-13-04; BI-55, D9)

- Anchors: `src/lib/chart-click.ts:50-75` (`ClickContext`; `layer` is its last field);
  `src/features/dashboards/tile-body.tsx:412-421` (the click of a combined chart);
  `src/features/dashboards/drill.tsx:14-26` (`DrillTarget`), `:141-144` (the panel's
  title); `src/features/dashboards/records.ts:12-44` (`RecordsRequest`), `:56-83`
  (`recordsQuery`); `src/features/dashboards/dashboard-page.tsx:1021-1026` (`setDrill`),
  `:1481` (`onRecords`); `src/features/dashboards/combined-tile.test.tsx:58-67` (the click
  test: the model).
- Change:
  1. `chart-click.ts`: `ClickContext` gains `question?: string`, documented: the legend
     name of the question the mark belongs to, set only by a chart with more than one
     question (`BI-55`). `LayerClick` is **not** changed (a test pins its exact shape).
  2. `tile-body.tsx`, inside the click handler at `:412-421` only: compute
     `question` as `layers[owner]?.label` when `owner >= 0`, else the `name` of
     `combined.option.series[hit.seriesIndex]` when the series is an array and has that
     index, else `undefined`; pass it in the context object beside `layer`. Touch no other
     line of the file (dashboards-p2 edits `:12` and after `:434`).
  3. `drill.tsx`: `DrillTarget` gains `question?: string`. In the panel, inside
     `DialogHeader` after the title, when `request.question` is set:
     `<p className="text-xs text-muted-foreground">Question: {request.question}</p>`.
  4. `records.ts`: `RecordsRequest` gains `question?: string`, documented as shown only and
     never sent. `recordsQuery` is not changed.
  5. `dashboard-page.tsx`: at `:1021-1026` add `question: ctx.question` to the object given
     to `setDrill`; at `:1481` add `question: drill.question` to the object given to
     `openRecords`. Nothing else.
- Check:
  - `combined-tile.test.tsx`, **inserted directly after the test at `:58-67`** (not at the
    end: dashboards-p2 adds a test there),
    **"tells a click the name of the question the mark belongs to"**: the same two clicks
    as the model; the first context's `question` is `"Rooms"`, the second's `"visitors"`.
  - `drill.test.tsx`: **"says which question the records belong to"** (a request with
    `question: "Rooms"` shows "Question: Rooms") and
    **"says nothing about a question for a chart with one"** (no text starting with
    "Question:").
  - `records.test.ts`, **"the question's name is shown, never sent"**: the query string of
    a request with `question: "Rooms"` does not contain `question` or `Rooms`.
  The first two fail on today's code; the last pins what must stay. The two lines in
  `dashboard-page.tsx` have no test of their own (a page test would need a click through a
  canvas); the typecheck covers the field names. Say so in the handoff.

### T11. The PDF is printed light, whatever the dashboard's theme (LH-BI-1-01 F2; BI-53, D15)

- Anchors: `src/features/dashboards/render-view.tsx:147` (`dashboardLook(data, false, …)`),
  `:164-169` (the chart image), `:185` (the PDF's box);
  `src/lib/dashboard-look.ts:44-61`; `src/features/dashboards/render-view.test.tsx:11-25`
  (the mocked `TileBody`), `:54-72` (`payload`), `:166-` (the chart image tests).
- Change, `render-view.tsx:147` only: the look is
  `dashboardLook(chart ? data : { theme: "light", font: data.font }, false, data.reporting?.fonts)`.
  Comment: QA `LH-BI-1-01` (`BI-53`): a dark box on the white page of a PDF leaves white
  bands at the margins and under the last tile, so the PDF is always light; a chart image
  is the whole picture and keeps the dashboard's theme. Update the component's doc comment
  with one sentence.
- Check, `render-view.test.tsx` (extend the mocked `TileBody` to also record its `dark`
  prop in a list that `afterEach` empties):
  - **"a dark dashboard is printed light: the page, the cards and the charts"**:
    `payload({ theme: "dark" })`: the `[data-render-view]` element has the class
    `theme-light` and not the class `dark`; every recorded `dark` is `false`.
  - **"a chart image keeps the dashboard's dark theme"**: the same payload with `chart`:
    the `[data-render-chart]` element has the class `dark`; the recorded `dark` is `true`.
  The first fails on today's code; the second pins what must stay.

### T12. The print view is laid out at the paper's width before any chart is drawn (LH-BI-1-01 F1, LH-BI-6-22, LH-BI-6-23; BI-53, D16, D17)

- **Before building**: read section 2 of `docs/plans/BI-53-RESULT.md` (T7). If it holds the
  last row of its table ("the plan's reading is wrong"), do **not** build this task: write
  that in the handoff and go to T13. In every other case, and also when the probe could
  not be run, build it: when the window already matches the paper it changes nothing that
  is seen.
- Anchors: `src/lib/render-summary.ts` (its exports: `filterLine`, `paramLines`,
  `issuedLine`); `src/features/dashboards/render-view.tsx:35` (`render.paper`,
  `render.orientation` in the payload), `:185` (the PDF's box);
  `src/app/globals.css:169-190` (the print rules; the two to repeat are `.dash-canvas`
  `:174-178` and `.dash-tile` `:182-189`); `rust/crates/lakehouse-api/src/pdf.rs:83-88`
  (the papers in inches), `:259-268` (the margins);
  `src/features/dashboards/dashboard-grid.tsx:120` (a canvas narrower than 640 px is drawn
  as the phone's column: every width below is wider).
- Change:
  1. `render-summary.ts`:
     `export function paperContentWidth(paper: string | undefined, orientation: string | undefined): number`.
     The paper's width in inches is 8.27 by 11.7 for `a4` and 8.5 by 11 for `letter`
     (anything else reads as `a4`); `landscape` takes the longer side, anything else the
     shorter; the answer is `Math.floor((side - 1) * 96)`: 697, 1027, 720, 960. Doc
     comment: the printable width in CSS pixels, from the sizes and the two 0.5 in margins
     of `lakehouse-api/src/pdf.rs`; whoever changes one changes the other.
  2. `pdf.rs`: one comment line at the margins (`:259`) and one at `inches` (`:83`) naming
     `paperContentWidth` in `src/lib/render-summary.ts`. No code change.
  3. `render-view.tsx:185`: the box gets the class `mx-auto` and the style
     `{ ...look.style, width: paperContentWidth(data.render?.paper, data.render?.orientation) }`.
     The chart image's box (`:169`) is not changed.
  4. `globals.css`, directly after the `@media print` block: the same declarations as the
     two print rules, outside any media query, for the print view only:
     `[data-render-view] .dash-canvas { … }` with the three declarations of `:175-177`, and
     `[data-render-view] .dash-tile { … }` with the declarations of `:183-188` plus
     `transition: none !important;`. Comment above them: `BI-53` (QA `LH-BI-1-01`,
     `LH-BI-6-22`, `LH-BI-6-23`): a chart is a canvas sized when it is created, so the print
     view is stacked in every medium, not only under `@media print`; these two rules repeat
     the print rules above and must stay the same as them. The existing print rules are not
     changed (the browser's own Print of the dashboard page uses them).
- Check:
  - `src/lib/render-summary.test.ts`,
    **"the printable width is the paper's width less the two side margins, in pixels"**:
    `("a4", "portrait")` 697; `("a4", "landscape")` 1027; `("letter", "portrait")` 720;
    `("letter", "landscape")` 960; `(undefined, undefined)` 697.
  - `render-view.test.tsx`, **"the print view is as wide as the paper's printable width"**:
    the default payload (A4 portrait): the `[data-render-view]` element's `style.width` is
    `697px`; with `render: { paper: "a4", orientation: "landscape" }` it is `1027px`.
  - `pdf.rs`: the existing test `the_request_carries_the_page_the_paper_the_wait_and_a_footer`
    passes unchanged.
  Both new tests fail on today's code. The stylesheet rule has no test (a unit test does
  not lay a page out), and no PDF of the real print view is made on the build machine:
  write both as *not verified* in the handoff. The owner runs `LH-BI-1-01`, `LH-BI-6-22`
  and `LH-BI-6-23` on staging after the merge.

### T13. The feature pages and the changelog say the new rules (BI-55, BI-53)

Every line this task writes on a feature page ends with the mark given with it. Never
write that the product owner signed or accepted any of this. In the lines below, `\``
stands for a plain backtick. No acceptance row changes on any page. `BACKLOG.md`,
`PRODUCT.md` and `HANDOFF.md` are the planner's.

- `docs/core/features/chart-builder-questions.md`, "Limits to tell a customer", as the
  **first** bullet of the list (dashboards-p2 adds one at the end):
  `- A sum is a whole number. An average, a minimum and a maximum keep up to six decimals; with no number format a value shows about four significant digits (1,466; 146.6; 14.66), and with decimals set it shows exactly those. A chart saved before this rule, with an average, a minimum or a maximum for the whole chart, shows whole numbers until it is saved again, unless a dashboard filter applies to it. *(proposed, not yet signed; \`BI-55\`)*`
- `docs/core/features/bins-groups-hierarchies.md`, Decisions table, a new row after row 6:
  `| 7 | Automatic and counted bins of a column whose values are all whole numbers are never narrower than 1, and a bin of width 1 over such a column reads as its one value ("2020"), not as a range | Planner default (QA round, \`BI-55\`) | *(proposed, not yet signed)* |`
- `docs/core/features/export-pdf-excel.md`:
  - Decisions table, row 7, replace its last sentence ("Cells that start like a formula are
    neutralised in both (as the CSV export does today).") by:
    `A text that starts like a formula is never run: in CSV it gets a leading apostrophe; in Excel it is a text cell and holds the text as it is (changed 2026-11-12, \`BI-55\`, *(proposed, not yet signed)*).`
  - Decisions table, a new row after row 11:
    `| 12 | The PDF is printed in the light theme, whatever theme the dashboard has; a chart image keeps the dashboard's theme. The print view is laid out at the paper's printable width, so a chart is as wide as its card on the page | Planner default (QA round, \`BI-53\`), *(proposed, not yet signed)* |`
  - "Limits to tell a customer", a new last line:
    `- An Excel file keeps a text that starts with =, +, - or @ as text. If the reader saves that file as CSV with their own program, the new file is not guarded. *(proposed, not yet signed; \`BI-55\`)*`
- `docs/core/features/query-studio-results.md`, Decisions table, row 6, append to its first
  cell:
  ` In Excel a text cell holds the text as it is, without the apostrophe: a text cell is never run (\`BI-55\`, *(proposed, not yet signed)*).`
- `docs/core/features/chart-formatting.md`, "Limits to tell a customer", a new last line:
  `- A rupiah amount shows "Rp" in both number locales. Any other currency shows what the locale gives it, which is its code where the locale has no symbol for it (SGD in the English locale). *(proposed, not yet signed; \`BI-55\`)*`
- `docs/core/features/joins-without-sql.md`, "Limits to tell a customer", a new last line:
  `- A joined column reads \`table.column\` in the record list and in the header of a downloaded raw table; on a chart and in a table tile it reads by the column's own name. *(proposed, not yet signed; \`BI-55\`)*`
- `CHANGELOG.md`, under `## [Unreleased]`, as the first bullets of `### Fixed`:
  - `- Numbers and labels on charts (backlog \`BI-55\`): an average, a minimum and a maximum keep up to six decimals instead of being rounded to a whole number (a chart saved before keeps whole numbers until it is saved again, unless a dashboard filter applies); automatic bins of a whole-number column such as a year are never narrower than 1 and read "2020"; a donut's legend pages instead of being cut off; the Rows tables read "(No value)" and "(Empty)" as the chart does; the records panel of a chart with several questions names the question; a joined column reads \`table.column\` in the record list and in a downloaded header; an Excel text cell that starts with = no longer shows the guarding apostrophe (CSV keeps it); a rupiah amount reads "Rp" in the English number locale too, and "Currency, compact" is a choice of its own.`
  - `- The PDF of a dashboard is printed in the light theme and laid out at the paper's width, so a dark dashboard has no white bands and a chart is as wide as its card (backlog \`BI-53\`). What was measured is in \`docs/plans/BI-53-RESULT.md\`; nothing was changed for speed.`
  If T12 was not built (its stop condition), leave "and laid out at the paper's width" and
  "and a chart is as wide as its card" out of the second bullet and out of row 12 above,
  and say so in the handoff.
- Check: `git diff --stat` shows only the six pages and `CHANGELOG.md`; every added line on
  a feature page carries its mark.

## 3. Verification

Per commit, only for what the commit touched (AGENTS.md, "Keep build time down"). The rules
of the common brief hold: never `cargo clean`, never `cargo test --workspace`, never a bare
`cargo test -p lakehouse-api <filter>`; the two repository lints always; `git status` before
every commit, nothing generated staged; a handoff quotes each command with its counts and
writes *not verified* with the reason for anything not run (rule 7). **Section 3a replaces
the machine-specific commands of the common brief** (`cargo-locked`, `-j 2`, "integration
tests are left to CI"): the developer works on another machine, where the integration tests
this plan names are run.

Tests by task (each named in section 2):

| Task | Rust unit (`--lib`) | Integration (`--test`) | Console (`bun test`) |
| --- | --- | --- | --- |
| T1 | `lakehouse-bi`: the whole `--lib` once (eight pinned statements moved) | `chart_numbers` | none |
| T2 | `lakehouse-bi --lib bin::tests`; `lakehouse-api --lib bin_rewrite` | `chart_numbers`, `sec11_guard`, `route_auth` | `src/lib/chart-bins.test.ts` |
| T3 | `lakehouse-api --lib export::tests`, `--lib csv_export` | `query_download`, `sec11_guard`, `route_auth` | none |
| T4 | `lakehouse-bi --lib export_format` | `dashboard_export` | `src/lib/value-format.test.ts`, `src/features/dashboards/format-section.test.tsx` |
| T5 | `lakehouse-bi --lib joins::tests` | `dashboard_export`, `sec11_guard`, `route_auth` | `src/features/dashboards/drill.test.tsx` |
| T6 | `lakehouse-api --lib pdf_export` | `sec11_guard`, `route_auth` | none |
| T7 | none | `pdf_real_gotenberg` (ignored by default; the procedure of T7) | none |
| T8 | none | none | `src/features/dashboards/chart-option.test.ts` |
| T9 | none | none | `src/components/patterns/rows-table.test.tsx`, `src/features/dashboards/chart-shape-section.test.tsx`, `src/features/dashboards/tile-dialogs.test.tsx` |
| T10 | none | none | `src/features/dashboards/combined-tile.test.tsx`, `drill.test.tsx`, `records.test.ts` |
| T11 | none | none | `src/features/dashboards/render-view.test.tsx` |
| T12 | `lakehouse-api --lib pdf::tests` | none | `src/lib/render-summary.test.ts`, `src/features/dashboards/render-view.test.tsx` |
| T13 | none | none | none |

Always, per commit:

```bash
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status    # nothing generated staged
```

Nothing here is proven in a browser or on a real PDF of a dashboard. The owner runs
`LH-BI-2-02`, `LH-BI-6-04` (with Format > Number, decimals 2, and with no format),
`LH-BI-6-39`, `LH-BI-6-38`, `LH-BI-13-04`, `LH-BI-7-04`, `LH-BI-1-08`, `LH-BI-1-06`,
`LH-BI-1-01`, `LH-BI-6-22` and `LH-BI-6-23` on staging after the merge.

## 3a. Change of machine

- The developer works on the ARM64 build machine, in the checkout
  `/home/rismana/rantai/lakehouse-agents-4`, and only under `/home/rismana/rantai/`.
- Before the first cargo command of a shell:
  `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-4 CARGO_BUILD_JOBS=6`.
  Plain `cargo` (there is no `cargo-locked` on that machine).
- Per Rust commit:

  ```bash
  cd rust && cargo fmt --check
  cargo clippy -p <crate> --all-targets -- -D warnings      # each crate the commit touched
  cargo test -p <crate> --lib <name>                        # each named unit test or module of the table in section 3
  cargo test -p lakehouse-api --test <file>                 # each integration test file the commit touched (Docker runs there)
  cargo test -p lakehouse-api --test sec11_guard            # ALWAYS when lakehouse-api/src is touched (T2, T3, T5, T6)
  cargo test -p lakehouse-api --test route_auth             # ALWAYS when lakehouse-api/src is touched
  ```

  T1 touches `lakehouse-bi` and adds a test file to `lakehouse-api`: clippy both crates.
- Never `cargo test --workspace`, never a bare `cargo test -p lakehouse-api <filter>`,
  never `cargo clean`.
- Counts: each integration test binary of `lakehouse-api` also runs one test from
  `src/tls_proxy.rs`, so its count is one more than the file's own tests. (It comes in
  through `tests/common/mod.rs`; `dashboard_export` and `query_download` include it. If
  `chart_numbers`, which is modelled on a file that does not include it, reports exactly
  its own tests, quote that count and say so.)
- TypeScript: before the first check, `bun install --frozen-lockfile`. Per console commit
  `bun run typecheck && bun run lint && bun test <the test files the commit touched>`.
  `bun run test` (the whole console suite) once, before the handoff.
- A PDF render on that machine needs a browser binary with an ARM64 build (here: the
  `gotenberg/gotenberg:8.37.0` image of T7). If it is not there, write *not verified
  locally* with the first line of the error; do not install system packages.
- Start no container you do not remove (T7's container is removed by `docker stop`; the
  test containers of `chart_numbers`, `dashboard_export` and `query_download` are the test
  harness's own). Stop no process you did not start.
- Before the handoff, once, in the foreground, on the final commit:

  ```bash
  cd rust && cargo fmt --check
  cargo clippy -p lakehouse-bi --all-targets -- -D warnings
  cargo clippy -p lakehouse-api --all-targets -- -D warnings
  cargo test -p lakehouse-bi --lib
  cargo test -p lakehouse-api --test chart_numbers
  cargo test -p lakehouse-api --test dashboard_export
  cargo test -p lakehouse-api --test query_download
  cargo test -p lakehouse-api --test sec11_guard
  cargo test -p lakehouse-api --test route_auth
  cd .. && bun run typecheck && bun run lint && bun run test
  python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
  git status
  ```

- CI on the pull request (x86_64) is the final word.

## 4. Out of scope

- **Anything that makes the PDF faster** (S2). No cause of the 45 seconds is shown; S1
  measures. The stages of a real dashboard's PDF are not measured on the build machine
  (D18).
- **A sum of a decimal column** stays rounded to a whole number (D4).
- **Charts saved before T1** are not rewritten, and a plain chart is not rebuilt on every
  read (D4).
- **"As it is now" from 1,000 up** shows no decimals (`decimalsFor`), and a single-number
  card with no format shows whole numbers (D4).
- **A rule from a column's name** (a column called `tahun` or `year` starting unbinned),
  and the thousands separator in a range of years ("2,000 – 2,005") (D5).
- **The builder's small preview** (the second half of `LH-BI-6-39`).
- **A joined column's label on charts, pickers and table tiles**, and in the CSV of the
  "numbers behind this chart" dialog (`downloadRowsCsv`): they keep what they show today
  (D10). A base column whose own name holds `__` reads with a dot in the record list and,
  on a chart with joins, in a downloaded header: the console already reads such a name
  that way everywhere.
- **Choosing "Currency" without typing a code**: the server refuses a currency format with
  no code; the list entry of T4 fills IDR only for the compact amount.
- **Other currencies the English locale has no symbol for** (SGD, THB) keep their code
  (D12).
- **The browser's own Print** of a dark dashboard, and of a dashboard in a dark console:
  its cards stay dark on the white page. A canvas cannot be recoloured by a stylesheet.
- **The PDF's tile height** (360 px, one tile per landscape page) and what a section tile
  looks like in the PDF.
- **The chart image (PNG)**: not changed.

Follow-up lines for the planner to add to `docs/core/BACKLOG.md` (the developer does not
edit it):

- `BI-<next>` Task (S2 of `BI-53`): make the dashboard PDF faster, from the stage numbers
  in `docs/plans/BI-53-RESULT.md`. Under `BI-1`.
- `BI-<next>` Task: a sum of a decimal column keeps its decimals. Under `BI-6`.
- `BI-<next>` Task: a chart saved before `BI-55` shows the same decimals with and without
  a dashboard filter. Under `BI-6`.
- `BI-<next>` Task: the browser's Print of a dark dashboard prints light. Under `BI-1`.
- `BI-<next>` Task: the builder's preview of a chart with many categories is larger.
  Under `BI-6`.

## Open questions for the owner

None blocks the build; the plan builds the answer named first. None of the wording or the
decisions in this plan has been seen by the owner.

1. **Excel without the apostrophe** (`BI-55`, D11). Planned: an Excel text cell holds the
   text as it is; CSV and TSV keep the apostrophe. A reader who saves the workbook as CSV
   themselves gets an unguarded file. Recommendation: as planned (the cell is typed text
   and is never evaluated; the apostrophe changes the data). The other choice: keep the
   apostrophe in Excel too and close the finding as intended.
2. **Decimals with no format** (`BI-55`, D4). Planned: unchanged, about four significant
   digits, so the average of `LH-BI-6-04` still reads "1,466" until a format with decimals
   is set, and then reads "1,466.18". Recommendation: keep; the rule is one for every
   fractional number, not only averages. The other choice: two decimals from 1,000 up.
3. **Six decimals** for an average, a minimum and a maximum; **sums stay whole** in this
   slice. Recommendation: as planned, and the follow-up for sums.
4. **Charts saved before** keep whole numbers until saved again when no filter applies.
   Recommendation: accept for now; the follow-up makes both reads the same.
5. **Years** (`BI-55`, D5). Planned: no rule from a column's name; whole-number bins of
   width 1 read as the value. A year column over more than ten years is still in ranges
   with separators. Recommendation: as planned. The other choice: a column named `tahun` or
   `year` starts with "No bins".
6. **"Rp" for IDR only** (D12). Recommendation: as planned; the narrow symbol for every
   currency would show "$" for SGD, CAD and AUD.
7. **The entry "Currency, compact (1.2 M)"** and the line **"Question: <name>"**, where the
   chart's own question is named by the clicked series. Recommendation: as planned.
8. **A donut's legend is one paged line** (D6), not several wrapped lines.
   Recommendation: as planned (it is what the rose and the funnel do).
9. **The PDF is always light** (`BI-53`, D15); a chart image keeps the dashboard's theme;
   the browser's Print is not changed. Recommendation: as planned.

## 5. Handoff

### Commits

Branch `fix/qa-round-dashboards-p3`, base `de9148b8a`, pushed after every
commit. One task per commit, in plan order; T3 is not among them.

| Commit | Title |
| --- | --- |
| `de4ba2bd` | fix(bi): an average, a minimum and a maximum keep six decimals (T1, LH-BI-6-04, BI-55) |
| `6a7fca68` | fix(bi): bins of a whole-number column are never narrower than 1 (T2, LH-BI-2-02, BI-55) |
| `931bec10` | fix(bi): a rupiah amount reads Rp in both locales, and compact money is a choice (T4, LH-BI-1-06, BI-55) |
| `56d83a53` | fix(bi): a joined column reads table.column in the record list and a downloaded header (T5, LH-BI-7-04, BI-55) |
| `77369b91` | fix(api): the API logs how long each stage of a PDF took (T6, LH-BI-1-01 F3, BI-53) |
| `bf9cd69f` | test(api): probe the renderer for its window, its media and its time (T7, LH-BI-1-01, LH-BI-6-22, LH-BI-6-23, BI-53) |
| `1cf81018` | fix(dashboards): a donut's legend pages instead of running over the chart (T8, LH-BI-6-39, BI-55) |
| `f6e0426e` | fix(dashboards): the Rows tables name a missing and an empty value as the chart does (T9, LH-BI-6-38, BI-55) |
| `35fdb75d` | fix(dashboards): the records panel says which question the records belong to (T10, LH-BI-13-04, BI-55) |
| `c94bac08` | fix(dashboards): the PDF is printed light, whatever the dashboard's theme (T11, LH-BI-1-01 F2, BI-53) |
| `aade8536` | fix(dashboards): the print view is laid out at the paper's width before a chart is drawn (T12, LH-BI-1-01 F1, LH-BI-6-22, LH-BI-6-23, BI-53) |
| `62418fa3` | docs(features): the feature pages and the changelog say the new rules (T13, BI-55, BI-53) |

**T3: not built, held by the planner for the owner's answer.** No code, no
test, no commit; the lines of T13 that describe the Excel text cell are left
out (see T13 below).

### MISMATCH: T2's engine expectation for a year column

The plan expects the binned statement over five years (2020 to 2024, width 1)
to answer **five** rows, `b` 2020 to 2024, each `n` 1. Measured on
`ClickHouse` 26.8 through `Range::bin_expr` and `Range::scalars_sql`, it
answers **four**: 2020, 2021 and 2022 once each and 2023 twice.

Cause, read in `bin.rs`: `bin_expr` clamps the last bin to
`ceil(mx / w) - 1`, so when the maximum sits on a bin edge the last bin's
lower edge is `mx - w` — the 2023 bin covers `[2023, 2024]` and holds both
years. That is `BI-10`'s own rule ("the maximum is never alone in a bin of its
own") meeting the new width-1 rule; the plan did not account for it. So a
five-year column reads four bars under "Automatic", the last one labelled
"2023" and holding two years, where D5 says it reads 2020 to 2024.

I did not change `bin_expr` and did not change the expected value. The test
`automatic_bins_of_a_year_column_are_one_year_wide_and_a_fractional_column_keeps_its_width`
builds the statement and runs it (so the measurement is repeatable) and
asserts only that every row of the table lands in exactly one bin (the sum of
`n` is 5), with the finding written at the site. **The planner decides**: either
`bin_expr`'s last-bin rule is excepted for whole-number bins of width 1, or D5
and its feature-page line say four bars. Everything else T2 asks for — width
1, `whole` on the wire, the label "2020" — is built and measured.

### Commands run, with counts

Every line ran in the foreground, with
`CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-4 CARGO_BUILD_JOBS=6`.
`bun install --frozen-lockfile` ran once before the first TypeScript check
(`Checked 779 installs across 753 packages (no changes)`). Before the first
`cargo test --test`, `docker ps -a` showed three `postgres:16-alpine`
containers from another session (`dreamy_dhawan`, `upbeat_tu`,
`upbeat_engelbart`); I started only `bi53-gotenberg-scratch` (T7) and removed
it with `docker stop`.

Per task, before the fix (the same test on the code without it; the working
tree held the test, and the fix was stashed where the two are in different
files):

| Task | Before the fix | After |
| --- | --- | --- |
| T1 | `cargo test -p lakehouse-bi --lib a_sum_is_a_whole_number` → 0 tests, 640 filtered out (the function and the test are the commit); `cargo test -p lakehouse-api --test chart_numbers` → 0 passed, **1 failed** (`the saved statement keeps the average's decimals: … round(sum(v)) AS __m0, round(avg(v)) AS __m1, round(max(v)) AS __m2 …`) | 1 passed, 640 filtered out; `chart_numbers` 1 passed |
| T2 | `bun test src/lib/chart-bins.test.ts` with the old `chart-bins.ts` → 8 pass, **3 fail** | 11 pass, 0 fail |
| T4 | `bun test src/lib/value-format.test.ts` with the old `value-format.ts` → 24 pass, **1 fail**; `bun test src/features/dashboards/format-section.test.tsx` with the old `format-section.tsx` → 14 pass, **2 fail** | 25 pass; 16 pass |
| T5 | `bun test src/features/dashboards/drill.test.tsx` with the old `drill.tsx` → 8 pass, **1 fail** | 9 pass, 0 fail |
| T6 | no test (the change is four log lines) | see below |
| T7 | n/a (a measurement) | see below |
| T8 | `bun test src/features/dashboards/chart-option.test.ts` with the old `chart-option.ts` → 17 pass, **1 fail** | 18 pass |
| T9 | the three files with the old sources → 30 pass, **3 fail** | 33 pass |
| T10 | `combined-tile.test.tsx` + `drill.test.tsx` with the old sources → 15 pass, **2 fail** | 30 pass (three files) |
| T11 | `render-view.test.tsx` with the old `render-view.tsx` → 9 pass, **1 fail** | 10 pass |
| T12 | `render-summary.test.ts` + `render-view.test.tsx` with the old sources → 10 pass, **2 fail, 1 error** (the missing export) | 15 pass |

Per task, the checks of sections 3 and 3a for what the commit touched:

- T1: `cargo fmt --check` OK; `cargo clippy -p lakehouse-bi --all-targets -- -D warnings` OK; `cargo clippy -p lakehouse-api --all-targets -- -D warnings` OK; `cargo test -p lakehouse-bi --lib` → **637 passed, 0 failed, 4 ignored**; `cargo test -p lakehouse-api --test chart_numbers` → 1 passed.
- T2: `cargo test -p lakehouse-bi --lib bin::tests` → **15 passed, 627 filtered out**; `cargo test -p lakehouse-api --test chart_numbers` → 2 passed; `cargo test -p lakehouse-api --lib bin_rewrite` → **7 passed, 3630 filtered out**; `cargo test -p lakehouse-api --test sec11_guard` → 4 passed; `--test route_auth` → 94 passed; `bun test src/lib/chart-bins.test.ts` → 11 pass.
- T4: the formatter oracle (the `bun -e` script of T4) before any edit → **`0 lines differ`**, and after the case file was edited → **`0 lines differ`**; `cargo test -p lakehouse-bi --lib export_format` → **5 passed, 637 filtered out**; `cargo test -p lakehouse-api --test dashboard_export` → 11 passed; `bun test src/lib/value-format.test.ts src/features/dashboards/format-section.test.tsx` → 41 pass.
- T5: `cargo test -p lakehouse-bi --lib joins::tests` → **11 passed, 632 filtered out**; `dashboard_export` → 11 passed; `sec11_guard` → 4; `route_auth` → 94; `bun test src/features/dashboards/drill.test.tsx` → 9 pass.
- T6: `cargo clippy -p lakehouse-api --all-targets -- -D warnings` OK; `cargo test -p lakehouse-api --lib pdf_export` → **3 passed, 3634 filtered out**; `sec11_guard` → 4; `route_auth` → 94.
- T7: `cargo clippy -p lakehouse-api --all-targets -- -D warnings` OK; `cargo test -p lakehouse-api --test pdf_real_gotenberg` → **0 passed, 4 ignored** (the new test is ignored by default); the probe's own run: **1 passed, 0 failed, 3 filtered out**, output below.
- T8–T12 (console): `bun run typecheck` → 0 errors; `bun run lint` → **0 errors, 11 warnings** (the same eleven that `main` has); the test files named above. T12 also `cargo test -p lakehouse-api --lib pdf::tests` → **8 passed, 3629 filtered out**.
- Every commit: `python3 ops/lint/check_bare_iceberg_count.py` (OK) and
  `python3 ops/lint/check_intra_package_imports.py` (OK), and `git status`
  showed only the files the task touched.

The before-handoff block of section 3a, on the final commit (`62418fa3`),
foreground:

```text
cd rust && cargo fmt --check                                        # OK
cargo clippy -p lakehouse-bi --all-targets -- -D warnings           # OK
cargo clippy -p lakehouse-api --all-targets -- -D warnings          # OK
cargo test -p lakehouse-bi --lib                                    # 639 passed; 0 failed; 4 ignored
cargo test -p lakehouse-api --test chart_numbers                    # 2 passed; 0 failed
cargo test -p lakehouse-api --test dashboard_export                 # 11 passed; 0 failed
cargo test -p lakehouse-api --test query_download                   # 24 passed; 0 failed
cargo test -p lakehouse-api --test sec11_guard                      # 4 passed; 0 failed
cargo test -p lakehouse-api --test route_auth                       # 94 passed; 0 failed
cd .. && bun run typecheck                                          # 0 errors
bun run lint                                                        # 0 errors, 11 warnings (as main)
bun run test                                                        # 4237 pass, 1 skip, 0 fail, 4238 tests, 389 files
python3 ops/lint/check_bare_iceberg_count.py                        # OK
python3 ops/lint/check_intra_package_imports.py                     # OK
git status                                                          # clean
```

`cargo test -p lakehouse-api --test chart_numbers` reports exactly its own
tests (2), because the file does not include `tests/common/mod.rs` and so does
not run the `tls_proxy` test; `dashboard_export` reports 11 and
`query_download` 24, one more than each file's own tests.

### Tests added, by name

- T1: `specs::tests::a_sum_is_a_whole_number_and_an_average_a_minimum_and_a_maximum_keep_six_decimals` (`lakehouse-bi`); `an_average_keeps_its_decimals_and_a_maximum_of_whole_numbers_stays_whole_on_the_engine` (`tests/chart_numbers.rs`, new file).
- T2: `bin::tests::a_column_of_whole_numbers_is_never_cut_narrower_than_one_and_the_tile_is_told`; `automatic_bins_of_a_year_column_are_one_year_wide_and_a_fractional_column_keeps_its_width` (`chart_numbers.rs`); `a bin of width 1 over whole numbers reads as its one value, with no separator`, `the tile's bins say whole only when the server said so`, `a year in a bin of one reads as the year` (`src/lib/chart-bins.test.ts`).
- T4: `a rupiah amount reads Rp in the English number locale too`, `a rupiah amount shown as its code still reads IDR, and other currencies keep their symbol` (`src/lib/value-format.test.ts`); `offers a compact amount of money as one choice and stores it as the compact style with the currency`, `shows a stored compact amount as that choice, and plain Compact drops the currency` (`src/features/dashboards/format-section.test.tsx`).
- T5: `joins::tests::a_joined_column_reads_table_dot_column_and_any_other_name_is_left_alone`; `a joined column is headed table.column in the list and in the one-record view` (`drill.test.tsx`).
- T6: none.
- T7: `the_renderer_reports_its_window_its_media_and_how_long_a_conversion_takes` (`tests/pdf_real_gotenberg.rs`, `#[ignore]`).
- T8: `a donut's legend pages when its entries do not fit, like a rose's and a funnel's` (`chart-option.test.ts`, inserted directly after the test the plan names).
- T9: `names a missing value (No value) and an empty one (Empty) when asked`, `leaves both blank when not asked` (`src/components/patterns/rows-table.test.tsx`, new file); `the Rows table under the preview names a missing and an empty category` (`chart-shape-section.test.tsx`); `the numbers behind a chart name a missing and an empty category` (`tile-dialogs.test.tsx`).
- T10: `tells a click the name of the question the mark belongs to` (`combined-tile.test.tsx`, inserted directly after the test the plan names); `says which question the records belong to`, `says nothing about a question for a chart with one` (`drill.test.tsx`); `the question's name is shown, never sent` (`records.test.ts`).
- T11: `a dark dashboard is printed light: the page, the cards and the charts`, `a chart image keeps the dashboard's dark theme` (`render-view.test.tsx`).
- T12: `the printable width is the paper's width less the two side margins, in pixels` (`render-summary.test.ts`); `the print view is as wide as the paper's printable width` (`render-view.test.tsx`).

### Existing assertions edited (rule 2: because the rule changed, not to get green)

T1, nine fragments, each the exact new statement:

| File | Old | New |
| --- | --- | --- |
| `builder.rs` (`two_aggregations_of_one_column_are_two_result_columns`) | `round(avg(stay)) AS avg_stay` | `round(avg(stay), 6) AS avg_stay` |
| `builder.rs` (same test) | `round(max(stay)) AS max_stay` | `round(max(stay), 6) AS max_stay` |
| `builder.rs` | `round(avg(visitors)) AS __m1` | `round(avg(visitors), 6) AS __m1` |
| `builder.rs` | `round(avg(stay)) AS avg_stay` (the `FROM` statement) | `round(avg(stay), 6) AS avg_stay` |
| `builder.rs` | `round(avg(stay)) AS __m0` | `round(avg(stay), 6) AS __m0` |
| `builder.rs` | `round(max(visitors)) AS __s0` | `round(max(visitors), 6) AS __s0` |
| `store.rs` | `round(avg(visitors)) AS visitors` | `round(avg(visitors), 6) AS visitors` |
| `store.rs` | `round(avg(stay)) AS avg_stay` | `round(avg(stay), 6) AS avg_stay` |
| `store.rs` | `round(avg(capacity__rooms)) AS capacity__rooms` | `round(avg(capacity__rooms), 6) AS capacity__rooms` |

`builder.rs:979`'s doc comment also changed: `round(avg(v)) AS avg_v` →
`round(avg(v), 6) AS avg_v`. No `round(sum(` line changed anywhere, and
`grep -rn 'round(avg(\|round(max(\|round(min(' rust/crates src ops` shows only the
six-decimal form (and the three expected strings in `specs.rs`).

T4 (three assertions and the case file):

| File | Old | New |
| --- | --- | --- |
| `export_format.rs` (`idr_compact_two_decimals_is_a_three_tier_excel_format`, en-US half) | `[>=1000000000]"IDR "#,##0.00,,," B";…` | `[>=1000000000]"Rp "#,##0.00,,," B";…` |
| `tests/dashboard_export.rs` | `styles.contains("IDR ")` | `styles.contains("Rp ")` |
| `tests/dashboard_export.rs` | `email,amount` + CRLF + `***,IDR 1.23 M` + CRLF, behind a byte-order mark | the same with `***,Rp 1.23 M` |
| `export_format_cases.jsonl` | 40 `en-US`/IDR/not-code lines with a leading `IDR ` or `-IDR ` | `Rp ` / `-Rp `; nothing else in the file moved (`40 insertions(+), 40 deletions(-)`) |

The case with `currencyDisplay: "code"` stays `"IDR "#,##0`, unchanged.

T5 (one assertion):

| File | Old | New |
| --- | --- | --- |
| `tests/dashboard_export.rs` | `email,mart_y__email,mart_y__secret` | `email,mart_y.email,mart_y.secret` |

### The probe (T7)

Container `gotenberg/gotenberg:8.37.0`, ARM64, on a container that had just
started, 2026-11-13. The test's own output:

```text
A4 landscape: 1966 ms for the conversion, 2 reports
  GET /probe?stage=start&w=780&box=780&print=false HTTP/1.1
  GET /probe?stage=later&w=780&box=780&print=true HTTP/1.1
A4 portrait: 1721 ms for the conversion, 2 reports
  GET /probe?stage=start&w=780&box=780&print=false HTTP/1.1
  GET /probe?stage=later&w=780&box=780&print=true HTTP/1.1
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
```

Read against the plan's table: `print` is `false` at `start` (F1's mechanism),
and the box is 780 px — narrower than the landscape page (1027, part empty)
and wider than the portrait page (697, cut at the card's right edge:
`LH-BI-6-22`, `LH-BI-6-23`). The last row of that table ("the plan's reading is
wrong") is **not** what was seen, so **T12 was built**. The numbers and the
*not measured* stages are in `docs/plans/BI-53-RESULT.md`, written in T7's
commit.

### Not verified

- **T6's four log lines on a running stack**: *not verified*. The crate has no
  log capture in its tests and no product stack runs on the build machine
  (D18); the planner reads them from staging after the merge.
- **T7's stages of a real dashboard's PDF**: *not measured* — slot wait, the
  renderer's start and page load, the dashboard's read, and draw/wait/print.
  Same reason; `docs/plans/BI-53-RESULT.md` section 4 says how to read them
  from one PDF of `LH-BI-1-01` on a deployment that runs this branch.
- **T12's stylesheet rule**: *not verified*. A unit test does not lay a page
  out, and no PDF of the real print view was made on the build machine. The
  owner runs `LH-BI-1-01`, `LH-BI-6-22` and `LH-BI-6-23` on staging.
- **T9's `add_bins`**: no test drives it — it needs the engine, the gate and
  the policy store together, and none exists today. The read's `whole` column
  is proven on the engine and the console's reading of it by unit tests; the
  route's insertion is covered only by the typecheck and the existing
  `dashboard_export` tests.
- **T10's two `dashboard-page.tsx` lines**: no test of their own (a page test
  would need a click through a canvas); the typecheck covers the field names.
- **T2's planned engine answer**: see the MISMATCH above; the plan's expected
  five rows are not asserted.
- Nothing here was seen in a browser or on a real PDF of a dashboard: the owner
  runs the QA cases of section 3 on staging after the merge.

### T13, as built

The five feature pages and `CHANGELOG.md` (the plan says "the six pages": the
sixth is `query-studio-results.md`, whose only line in T13 describes the Excel
text cell, held with T3). Every added line carries its
`*(proposed, not yet signed)*` mark; nothing says the owner accepted anything.
Left out with T3: the row 7 sentence and the last limit of
`export-pdf-excel.md`, the appended sentence of `query-studio-results.md` row 6,
and the Excel clause of the first changelog bullet. Row 12 of
`export-pdf-excel.md` keeps "and laid out at the paper's width" and "and a
chart is as wide as its card" because T12 was built.

## 6. Review

### Round 1 (planner, 2026-11-13, at `b11fe6f4`)

Reviewed against sections 1 to 4 (an independent reviewer's notes, then the planner's
rulings). The Rust code was read, not built, on the planner's machine; CI on the pull request
(#503) is the run. The console was run in a clean checkout of the branch: `bun run
typecheck` 0 errors; `bun run lint` 0 errors, 11 warnings; `bun run test` 4237 pass, 1 skip,
0 fail across 389 files; the new tests fail against `origin/main`'s sources (16 of them),
as intended. T1 and T4 to T13 match the plan. No existing test was loosened; the pinned
statements changed are avg, min and max only, each to its exact new text; the 80 changed
lines of `export_format_cases.jsonl` are IDR rows whose symbol becomes "Rp". Nothing of T3
is in the diff. No reference is invented.

**The planner's decision D5 was wrong, and T2 comes out of this pull request.**

- **R1. BLOCKER: the last bar of a whole-number column is labelled as one value and holds
  two.** For a year column over 2020 to 2024 the new floor makes the width 1; the bins are
  `[2020, 2021)`, `[2021, 2022)`, `[2022, 2023)`, `[2023, 2024]`, because the maximum
  belongs to the last bin (`bin_expr`, `BI-10`); the console then labels that bin "2023",
  and a click on it lists 2023 and 2024. The developer measured exactly this on the engine
  (four rows, 2023 twice) and said so. On `main` the same chart has five bars with ugly but
  true labels ("2,020 – 2,020.5"). So T2 as planned replaces an ugly label by a wrong one,
  for every whole-number column with a range of 1 to 10. The floor only matters for those
  small ranges, so there is no part of T2 worth keeping without a bin of its own for the
  maximum, and that needs a fourth scalar in the statement, a changed `bin_expr`, the same
  exception in `bin_records_predicate` and in the console's last-bin rule, and new pinned
  statements: a plan of its own. **Ruling: undo T2 completely** (rule 14: unsupported,
  honestly, over works, approximately). `LH-BI-2-02` stays open on `BI-55`.
  - Undo everything commit `6a7fca68` changed, and nothing else: the floor and `whole` in
    `rust/crates/lakehouse-bi/src/bin.rs`, `add_bins` in
    `rust/crates/lakehouse-api/src/routes/support.rs`, `binLabel`/`relabelBins` and the
    `whole` field in `src/lib/chart-bins.ts`, the tests it added to
    `src/lib/chart-bins.test.ts`, and its test in
    `rust/crates/lakehouse-api/tests/chart_numbers.rs` (T1's test in that file stays).
    Start from `git revert --no-commit 6a7fca68`; if a later commit touched the same lines,
    resolve by hand so that the later commit's own change stays. Afterwards
    `git diff origin/main...HEAD -- rust/crates/lakehouse-bi/src/bin.rs src/lib/chart-bins.ts src/lib/chart-bins.test.ts`
    must show nothing that T2 added (say in the handoff what, if anything, is left and
    which task it belongs to), and the pinned bin statements are again `main`'s.
  - Documents (T13 wrote them): remove row 7 of the Decisions table in
    `docs/core/features/bins-groups-hierarchies.md` (the whole row this branch added), and
    in `CHANGELOG.md` remove from the `BI-55` entry the clause "automatic bins of a
    whole-number column such as a year are never narrower than 1 and read "2020";" and
    nothing else. If another document line of this branch speaks of whole-number bins,
    remove that line too and name it in the handoff.
- **R2. BLOCKER, closed by R1:** the engine test of the bins asserted `sum(n) == 5`, which
  cannot fail, under a doc comment that said "five years, one per bin". It goes with T2.
- **R3. SHOULD-FIX: `render_ms` counts the wait for a slot.** `routes/pdf_export.rs`: one
  `Instant` is taken before `take_slot` and used for both lines; the plan says "`render_ms`
  counts from the slot". After the slot is taken: `let rendering =
  std::time::Instant::now();` and `render_ms` is measured from it. In `routes/render.rs`
  call the existing `millis` helper where its body is written out again.
- **R4. SHOULD-FIX: `docs/plans/BI-53-RESULT.md`.** Remove the checkout's path (a
  measurement file names the machine by its architecture and CPU count, not by a path with
  an account's name). Beside "The first minus the second, 245 ms, is the browser's start"
  write "(one run each, and the two runs differ in orientation as well as in order)".
- **R6. SHOULD-FIX (rule 13):** `src/features/dashboards/render-view.tsx`, where the print
  view's width is set, gets a one-line comment:
  `// QA LH-BI-1-01 F1, LH-BI-6-22, LH-BI-6-23 (BI-53, D17): laid out at the paper's width before a chart is drawn.`
  and the doc of `paperContentWidth` (`src/lib/render-summary.ts`) names the same three
  cases.
- **R5. Note:** the handoff says "T9's `add_bins`"; it is T2's, and it leaves with T2.
- **R7. For the planner (no change here):** `rust/crates/lakehouse-alerts/src/lib.rs`
  (`mart_statement`) still writes `round({agg}(x))`, so an alert on an average is checked
  against a whole number while the chart now shows decimals. It goes into the follow-up
  slice with the bins (`BI-55`, part 2), together with a sum of a decimal column.

All of this is ONE commit, `fix(review): round 1 of dashboards-p3: T2 is undone (R1, R2), R3, R4, R6`,
after `main` is merged into the branch. Its body says why T2 is undone, in two sentences.

The developer's handoff for this round, appended below this entry: the merge commit and the
fix commit; what the revert had to resolve by hand; `cargo fmt --check`; `cargo clippy -p
lakehouse-bi --all-targets -- -D warnings` and the same for `lakehouse-api`; `cargo test -p
lakehouse-bi --lib` (quote the counts; the bin tests are `main`'s again); `cargo test -p
lakehouse-api --test chart_numbers`; `--test dashboard_export`; `--test sec11_guard`;
`--test route_auth`; `bun run typecheck`; `bun run lint`; `bun test src/lib
src/features/dashboards`; `bun run test` once; each with its counts.

### Round 1, developer's handoff

**Merge and fix commits.**

- Merge of `main`: none written. `git fetch origin` left `origin/main` at `43d36f55`, which
  is already the merge base of the branch and an ancestor of `HEAD` (`git rev-list --count
  HEAD..origin/main` = 0); `git merge origin/main` said "Already up to date." and there was
  nothing to merge, so there is no merge commit and no conflict to resolve. The step A
  checks ran on `58649f30` (the review commit) and the push was a no-op
  ("Everything up-to-date").
- Fix commit: `899adfd7` — `fix(review): round 1 of dashboards-p3: T2 is undone (R1, R2), R3, R4, R6`.
- Handoff commit: the one that adds this section, `docs(plan): handoff for review round 1 of dashboards-p3`.

**What the revert had to resolve by hand.**

- Nothing. `git revert --no-commit 6a7fca68` applied cleanly, and
  `git log 6a7fca68..HEAD -- <the five files>` is empty, so no later commit touched those
  lines. The revert removed the `fr` column, the `greatest(…, if(fr = 0, 1, 0))` floor,
  `WidthRule::is_nice`, the `whole AS whole` read, its `bin.rs` test, the `whole` key in
  `add_bins`, and the `chart-bins.ts`/`chart-bins.test.ts` changes; those two console files
  are byte for byte `main`'s again. T1's test in `chart_numbers.rs` stays; T2's test and its
  `default.bi55_years` fixture are gone. `whole` has no reader left: `grep -rn '"whole"' src/ rust/ dagster/`
  finds nothing, and the three files R1 names keep only the English word in older comments
  (`bin.rs:305`, `support.rs` doc lines), none of them T2's.
- `git diff origin/main...HEAD -- rust/crates/lakehouse-bi/src/bin.rs src/lib/chart-bins.ts src/lib/chart-bins.test.ts`
  shows one hunk, and it is not T2's: `bin.rs:595` (`grid_points_sql`) uses
  `agg.rounded(measure)` with the `LH-BI-6-04 (BI-55, D4)` mark — T1's change. Nothing else
  is left.
- Documents: row 7 of the Decisions table in `docs/core/features/bins-groups-hierarchies.md`
  is removed, and the `BI-55` changelog entry loses only the clause "automatic bins of a
  whole-number column such as a year are never narrower than 1 and read "2020";". No other
  line of this branch speaks of whole-number bins: `grep` over `docs/`, `CHANGELOG.md`,
  `src/` and `rust/` finds only the plan's own T2 section and the pre-existing
  `chart-builder-questions.md:57` (T1's sum rule) and `bin.rs:305` (a width parse), none of
  them about bins of a whole column.

**R3, R4, R6.**

- R3: `make_pdf` keeps `started` for `slot_wait_ms` and takes
  `let rendering = std::time::Instant::now();` after `take_slot`; `render_ms` uses it, with
  the R3 mark at the line. `pdf_export::millis` became `pub(crate)` and
  `routes/render.rs` calls it for `payload_ms` instead of repeating its body. The
  `slot_wait_ms`/`render_ms` lines are unchanged as log keys, so the plan's stage names in
  `BI-53-RESULT.md` still match.
- R4: the machine line reads "ARM64 (`aarch64`), 20 CPUs." with no path; the 245 ms sentence
  carries "(one run each, and the two runs differ in orientation as well as in order)". No
  number changed.
- R6: the one-line comment sits above a `const width = paperContentWidth(…)` in
  `render-view.tsx` (a `//` comment in that spot of the JSX tree would print as text), and
  the box's style uses `width`; its value and the test are unchanged. `paperContentWidth`'s
  doc names `LH-BI-1-01` F1, `LH-BI-6-22` and `LH-BI-6-23`.
- R5 needed nothing (the plan's own sentence is the fix). T3 stays not built.

**Commands, run in the foreground on `899adfd7`, `CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-4
CARGO_BUILD_JOBS=6`.**

| Command | Result |
| --- | --- |
| `cargo fmt --check` (from `rust/`) | clean, no diff |
| `cargo clippy -p lakehouse-bi --all-targets -- -D warnings` | `Finished` in 7.11 s, 0 warnings |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | `Finished` in 1 m 12 s, 0 warnings |
| `cargo test -p lakehouse-bi --lib` | 638 passed, 0 failed, 4 ignored |
| `cargo test -p lakehouse-api --test chart_numbers` | 1 passed, 0 failed (T1's only; T2's is gone) |
| `cargo test -p lakehouse-api --test dashboard_export` | 11 passed, 0 failed |
| `cargo test -p lakehouse-api --test sec11_guard` | 4 passed, 0 failed |
| `cargo test -p lakehouse-api --test route_auth` | 94 passed, 0 failed |
| `bun install --frozen-lockfile` | 779 installs, 753 packages, no changes |
| `bun run typecheck` | 0 errors |
| `bun run lint` | 0 errors, 11 warnings (the same 11 as before this round) |
| `bun test src/lib src/features/dashboards` | 1971 pass, 1 skip, 0 fail, 4361 expect() calls, 209 files |
| `bun run test` | 4234 pass, 1 skip, 0 fail, 13035 expect() calls, 389 files |

`bun run test` is 3 lower than the review's 4237 because T2's three console tests left with
T2; no other test changed.

**Not verified.**

- `cargo test` is not run in full on the branch: the block above is the scoped set of step C
  (rules: no `cargo test --workspace`). CI on the pull request is the full run.
- The `render_ms`/`slot_wait_ms` split is not seen in a live log: it needs a running stack
  and Gotenberg, which the build machine does not start (D18), the same reason
  `BI-53-RESULT.md` lists those stages as *not measured*.
- The print view's width and the R6 comment have no runtime check beyond
  `render-view.test.tsx`'s `style.width`; a real PDF is still the owner's acceptance run.
