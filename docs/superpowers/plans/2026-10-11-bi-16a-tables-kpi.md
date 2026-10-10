# BI-16 part A: tables and KPI cards — Implementation Plan

**Status:** planner defaults, not started. Written by the planner (Claude
Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/tables-and-kpi-cards.md`. Specs:
`docs/core/specs/bi-16.md` part A, `docs/core/specs/ai-5.md`.

**Base:** `feat/uiux` at `e70f9df`. Read first: the `BI-9` and `BI-18` part B
plans and their handoffs (grain, `TimeContext`, paged records, `truncated`),
`lakehouse-bi` `builder.rs` / `store.rs` (how `table`, `kpi` and `gauge` are
built: `spec_from_kpi_input` at `store.rs:1640`, the table kind near
`:1551`), `src/features/dashboards/tile-body.tsx` (`:51` KPI, `:61` table),
`drill.tsx` and `records.ts` (the record list).

**Verification on this slice (owner's instruction, 2026-10-11):** the owner
tests by hand. The developer runs the cheap checks only (section 6); the
reviewer does not re-run suites; CI runs them on the PR.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Raw table | The `table` kind gains `tableMode`: absent or `"grouped"` is today's behaviour, `"rows"` is raw rows. In rows mode the definition lists `columns` (1 to 30, each a column of the source) and an optional sort (`sortColumn`, `sortDir`). The tile is paged by the server: 50 rows a page, `offset`, `total`, through the same guard, role rewrite and dashboard filters as every tile. Prefer extending the existing records route and its console hook over a new route. Works for marts and SQL sources. In embeds and public links: the first page only. |
| Column settings | `columnSettings` on the definition, for both table modes and the pivot's values: per column `label`, `format` (`auto`, `number`, `percent`, `currency`, `date`, `link`, `image`), `decimals` (0 to 6), `width` (px, 60 to 800), `wrap`, `hidden`. Column order is the order of `columns`. Validated at save; unknown keys refused. Formatting happens in the console; the server stores and returns it. |
| Links and images | A `link` cell renders an anchor only for `http:` / `https:` values (`target="_blank"`, `rel="noopener noreferrer"`); an `image` cell renders `<img>` only for `https:` (`referrerPolicy="no-referrer"`, `loading="lazy"`, fixed maximum height). Any other value is plain text. Check the console's CSP allows it; if it does not, report rather than loosen it. |
| Pivot | New kind `pivot`: `rows` (1 to 3 columns), `columns` (0 to 2), `values` (1 to 5, each `{ column, aggregate }` using the existing aggregate list). `totals`: `none`, `grand`, `all` (grand plus subtotals). Totals come from the engine over the underlying rows (grouping sets / rollup), not from summing cells. A date field may carry a `BI-9` grain. At most 10,000 body cells; beyond that the first are returned with `truncated: true`. Collapsing a group is console-side state, not saved. |
| KPI comparison | Optional `compare` on a `kpi`: `{ kind: "previous", dateColumn, period }` with `period` in `day`, `week`, `month`, `quarter`, `year`, or `{ kind: "goal", value }`. Optional `goodDirection`: `up` (default) or `down`. For `previous` the server returns the latest period with data, the one before it, and up to 12 periods for the trend line, bucketed with `BI-9`'s `TimeContext`; the console shows amount and percent change, coloured by `goodDirection`, and a sparkline. A previous value of zero shows the amount and no percent. For `goal` it shows the distance and percent of goal. |
| Record detail | In the record dialog, a row opens a one-record view listing every field with its value, with Previous / Next that walk the list and cross page boundaries. Console only; no new route. |
| Assistant | `create_chart` / `update_chart` schemas gain `pivot`, `tableMode`, `columns`, `columnSettings`, the pivot fields, `compare`, `goodDirection`; `tool_schemas.json` regenerated. |
| Export | `chart_def_fields` (YAML export) lists the new fields. CSV export of a raw table exports what the existing table export does today; say what that is in the handoff. |

## 2. Tasks

Rust first, in one sitting; then console.

- **T1 — `lakehouse-bi`:** definition fields and validation; rows-mode SQL (selected columns, sort, page, count); pivot SQL with engine totals and the cell cap; KPI comparison SQL. Unit tests per shape.
- **T2 — API:** paged rows for a raw-table tile; pivot and comparison in the dashboard payload, preview, embed and public; export; assistant schemas and fixture.
- **T3 — Console, raw table:** tile with paging and header sort; builder controls for mode, columns (pick, reorder, hide) and per-column settings; cell formatters in `src/lib`.
- **T4 — Console, pivot:** builder (rows, columns, values, totals) and the tile: sticky headers, subtotals, collapse, cut-off mark.
- **T5 — Console, KPI:** builder controls and the tile (amount, percent, colour, sparkline).
- **T6 — Console, record detail.**
- **T7 — Docs:** `CHANGELOG.md` `[Unreleased]`.

## 3. Out of scope (do not build)

Parts B and C; cell colouring rules; paging inside embeds; calculated columns; editing data.

## 4. Things the developer must verify, not assume

- Each generated statement shape once against the dev `ClickHouse` (26.8): rows mode with sort and a filter, pivot with `all` totals on two row fields and one column field, KPI previous-period on a `Date` and on a `DateTime` column. Quote statement and result. (Tables: `serving.mart_demo_map_points`, 237 rows, `visit_date Date`; `serving.mart_demo_events`, 1440 hourly rows, `event_time DateTime('UTC')`. Read-only.)
- How the engine marks total rows (null versus empty string versus a grouping function), so a real null group is not mistaken for a total.
- That a raw table cannot select a column the role rewrite masks or denies without the mask applying.
- The console's CSP for `img-src`.

## 5. Checks (reduced, by the owner's instruction)

Per task: `cargo fmt --check`, `cargo clippy -p <crate> --all-targets -- -D warnings`, and the unit tests of the files touched; `bun run typecheck`, `bun run lint`, and the bun test files touched. At the end: one `cargo test -p lakehouse-bi`, one `cargo test -p lakehouse-api --lib`, `--test route_auth --test sec11_guard`, and one full `bun run test`. Nothing else.

## 6. Handoff (developer appends)

### Partial notes (developer, uncommitted)

- 2026-10-10 T1 done (not committed): `lakehouse-bi/src/tables.rs` (new: `TableFields` flattened into `ChartInput`, `ColumnSetting`, `PivotValue`, `Compare`, `plan_rows`/`rows_sql`, `plan_pivot`/`pivot_sql` with `GROUPING SETS` and `grouping()` flags, `plan_compare`/`kpi_trend_sql`), `ChartKind::Pivot`, `builder.rs` (`rebuild` dispatches the three shapes, `FilteredSql.table`, `rows_page_sql`), `store.rs` (save-time validation, `spec_from_rows_input`, `spec_from_pivot_input`, KPI comparison), `grain.rs` (`Pivot` takes a grain). `cargo test -p lakehouse-bi`: 204 lib + 1 pass; clippy `-p lakehouse-bi --all-targets -D warnings` clean.
- 2026-10-10 T2 done (not committed): API (`records` route takes `columns`/`sortColumn`/`sortDir`; `support::annotate_table` adds `total`/`limit`/`offset` to a raw table's tile and cuts a pivot at its cap with `truncated`; embed and public use it; preview and SQL-source preview annotate; YAML export lists the new fields; `create_chart`/`update_chart` schemas and `tool_schemas.json` regenerated, `pivot` added to the kind enum). Focused API tests pass (24, including a masked/sorted raw-table page); the single `--lib` and `route_auth`/`sec11_guard` runs are still to do at the end.
- 2026-10-10 T3 to T6 done (not committed): lib (`table-types.ts`, `cell-format.ts`, `pivot.ts`, `kpi-compare.ts`, `table-draft.ts`, each with a test file: 13 + 9 + 9 + 9), features (`raw-table.tsx`, `pivot-table.tsx`, `kpi-card.tsx`, `table-cell.tsx`, `table-settings.tsx`; `tile-body.tsx`, `chart-builder.tsx`, `drill.tsx`, `records.ts`, `dashboard-page.tsx` edited; `rows-table.tsx` takes an optional `onRowClick`), tests `table-tiles.test.tsx` (22) and three new `drill.test.tsx` tests (30 pass in the two files). `bun run typecheck` clean, `bun run lint` 0 errors and the same 6 warnings as before. T7 (CHANGELOG) written. The handoff and the final single runs follow.

### BI-16 part A, T1 to T7 — handoff (developer, 2026-10-10, uncommitted)

Nothing is committed, pushed or stashed. No migration, no Postgres change.

**Files by task**

- T1 `lakehouse-bi`: `src/tables.rs` (new), `src/builder.rs`, `src/store.rs`, `src/grain.rs`, `src/specs.rs` (`ChartKind::Pivot`), `src/lib.rs`.
- T2 `lakehouse-api`: `src/routes/dashboard.rs` (records route, YAML export, tests), `src/routes/support.rs`, `src/routes/embed.rs`, `src/routes/dashboard_sources.rs`, `src/routes/ai/registry.rs`, `tests/fixtures/tool_schemas.json`.
- T3 to T5 console: `src/lib/{table-types,cell-format,pivot,kpi-compare,table-draft}.ts` (+ tests), `src/features/dashboards/{raw-table,pivot-table,kpi-card,table-cell,table-settings}.tsx`, `tile-body.tsx`, `chart-builder.tsx`, `dashboard-page.tsx`, `records.ts`, `tile-dialogs.tsx`, `src/lib/{dashboard-specs,time-grain,chart-click,preview-key}.ts`, `src/services/clients/bi-store.ts`.
- T6: `drill.tsx` (one-record view), `src/components/patterns/rows-table.tsx` (optional `onRowClick`).
- T7: `CHANGELOG.md` `[Unreleased]`.

**Commands run, with counts (all in the foreground)**

- `cargo fmt --check -p lakehouse-bi -p lakehouse-api`: clean (I ran `cargo fmt -p` on both; the HEAD versions of every file I touched were already rustfmt-clean, so only my hunks moved).
- `cargo clippy -p lakehouse-bi --all-targets -- -D warnings` and `-p lakehouse-api --all-targets -- -D warnings`: clean (after `touch` of each `lib.rs`).
- `cargo test -p lakehouse-bi`: 210 lib + 1 pass.
- `cargo test -p lakehouse-api --lib`: 1574 passed, 0 failed, 1 ignored (the fixture writer). Before the plan: 1568.
- `cargo test -p lakehouse-api --test route_auth --test sec11_guard`: 32 + 4 pass. No route was added, so `POLICY_TABLE` is unchanged.
- `cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored` (once): regenerated `tool_schemas.json` (+170 lines: the new properties on both chart tools and `pivot` in the kind enum; nothing removed).
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (the same six as before). Full `bun run test`: 1201 pass, 1 skip, 0 fail (1202 tests, 134 files). Before: 1145.

**Real-engine statements (ClickHouse 26.8.9.10, read-only, no scratch objects created)**

- Rows mode, sort and filter: `SELECT place, provinsi, visitors FROM serving.mart_demo_map_points WHERE provinsi IN ('Jawa Barat','Bali') ORDER BY visitors DESC, place, provinsi, visitors LIMIT 5 OFFSET 0` returned Bandung #4 2224, Denpasar #4 2207, Bandung #3 2137, Bogor #7 2035, Bogor #6 1955; the count statement over the same `WHERE` returned 17. Last page: `ORDER BY visitors DESC, place, visitors LIMIT 3 OFFSET 234` returned 3 rows (47, 47, 44) and `count()` was 237. The same shape over a derived table with the `SETTINGS` cap ran and returned 38 rows for `visitors > 2000`.
- Pivot, rows `provinsi, category`, column `visit_date` bucketed by quarter (and by month), `all` totals: `SELECT provinsi, category, m, sum(visitors) AS __v0, count() AS __v1, grouping(provinsi) AS __g0, grouping(category) AS __g1, grouping(m) AS __g2 FROM (SELECT date_trunc('quarter', visit_date) AS m, provinsi, category, visitors FROM (SELECT * FROM serving.mart_demo_map_points WHERE visitors > 0) AS flt) AS bkt GROUP BY GROUPING SETS (...) ORDER BY grouping(provinsi, category, m) DESC, provinsi, category, m`. The first row (all flags 1) is `278955 / 237`, equal to `SELECT sum(visitors)` and `count()` on the table; per-quarter totals 18148 / 51437 / 47943 / 60020 / 58350 / 43057 add to 278955. With `avg` the grand total is 1177.0253164556962, equal to the table's own average. The same statement over a SQL-source derived table with the cap returned 717 rows.
- Total rows: on 26.8 a rolled-up `Nullable` key prints as `NULL` and a rolled-up `String` or `Date` key as its default (`''`, `1970-01-01`); a real `NULL` group has `grouping() = 0` (probe: `GROUP BY GROUPING SETS ((k),())` over a `Nullable(String)` with real nulls gave `\N 21 g=1` for the total and `\N 9 g=0` for the real null group). So neither null nor empty can mark a total: the `__g<j>` flags do, and the console reads only them.
- KPI previous period on a `Date`: `SELECT * FROM (SELECT visit_date, round(sum(visitors)) AS v FROM (SELECT date_trunc('month', visit_date) AS visit_date, visitors FROM (SELECT * FROM serving.mart_demo_map_points WHERE provinsi IN ('Bali','Jawa Barat')) AS flt) AS bkt WHERE visit_date IS NOT NULL GROUP BY visit_date ORDER BY visit_date DESC LIMIT 12) ORDER BY visit_date` returned ten monthly rows, oldest first, the last two 2026-10-01 6511 and 2026-11-01 629. On a `DateTime('UTC')` (`serving.mart_demo_events`, `date_trunc('week', toTimeZone(event_time, 'Asia/Jakarta'))`): ten weeks, 2026-07-27 655 to 2026-09-28 888, last two 2683 and 888. On a quarter bucket over a SQL-source derived table: six rows.
- A raw table and the role rewrite: the unit test `a_raw_table_page_masks_a_listed_and_a_sorted_column` (real policy row in Postgres, wiremock `ClickHouse`) lists a masked column and sorts by it; every statement sent, page and count, carries the `replaceRegexpOne(toString(`email`)...)` mask. Not run against a real masked mart.
- Console CSP: the console sets a `Content-Security-Policy` only on `/embed/*`, and it holds `frame-ancestors` and nothing else (`src/proxy.ts`, `src/lib/embed-frame.ts`); there is no `img-src`, so an `https` image loads. Nothing loosened.

**CSV export of a raw table:** `downloadRowsCsv` writes the tile's `columns` and `rows` as they came, so for a raw table it is the first page of 50 rows with every listed column, hidden ones included. Unchanged code. Whether a whole-result export is wanted is a decision for the planner (it would be a new route or a paged client loop).

**Existing tests changed**

- `src/lib/preview-key.test.ts`: `base` gained `tables: ""`. The test asserts that every input changes the key; the key has a new input (the table, pivot and comparison fields), so `base` had to list it. No assertion removed.
- `tests/fixtures/tool_schemas.json` regenerated by the existing ignored writer for a reviewed schema change (additions only).
- `src/features/dashboards/drill.test.tsx`: three tests added; none changed. `rows-table.tsx` gained an optional prop; its other uses are unchanged.

**Deviations and choices where the plan was silent**

1. The new fields are one `TableFields` struct flattened into `ChartInput` (`ChartInput.tables`); the wire shape is flat camelCase, as the plan says. A chart saved before reads back with every one absent.
2. `columns` means the listed columns of a raw table and, on a pivot, its column fields; `rows` is the pivot's row fields. A raw table or a pivot refuses `dimension`, `measures` and `breakdown`, and a kind refuses a field it does not take (as `lat`/`lon` are refused).
3. Totals: `grand` = every row total, every column total and the corner; `all` = a total for every leading prefix of the row fields and of the column fields (rows `[p, k]`, columns `[m]` gives six grouping sets). Subtotals fold in the console only with `all`, because a folded group stays in view as its subtotal.
4. A pivot carries one `grain`, applied to the first date or timestamp field among rows then columns; the dashboard's grain switch applies to a pivot like any other grained chart (a pivot is never cut to its "latest buckets"). `count` in a pivot is `count()` and ignores its column.
5. Cell cap: 10,000 divided by the number of values, long-format rows, and below 2,000 over a SQL source so `ClickHouse` never cuts by itself; the statement asks for one row more and the API trims and sets `truncated: true`. Totals are ordered first so the cut falls on body cells.
6. A raw table's later pages and header sort go through the existing records route (`columns`, `sortColumn`, `sortDir`), over the same `filter_predicates`. The records route ignores the legacy `year` query parameter, which the console never sends; the tile statement applies it.
7. The dashboard payload gives the raw table `total`, `limit` and `offset`; if the count statement fails it is logged and `total` is left out, and the tile then shows its rows and no paging. Embeds and public links show the first page and "First N of M rows". The builder preview shows the first page without a total.
8. KPI "previous period" is the last two periods that have data (a month with no rows is skipped, so "previous" can be two months back); the trend line is up to 12 such periods. The KPI value in that mode is the latest period's, not the whole table's.
9. `percent` format reads the value as a ratio (0.25 is 25%); `currency` is `Intl` `id-ID` IDR with 0 decimals unless set; `date` shows the calendar day only.
10. Column settings keys must be columns of the relation; for a grouped table the builder offers the dimension and the measure.

**Not verified**

- Nothing was seen in a browser. The builder dialog is not rendered by any test (as for BI-9); `previewKey` includes the table fields and the draft conversions are pinned by `table-draft.test.ts`.
- The workspace-wide clippy and test, the other `lakehouse-api` integration test files, `docker compose`, and the gates (CI runs them).
- A raw table, pivot or comparing KPI on an embed or a public link end to end (only the shared `annotate_table` path is covered by the dashboard payload tests).
- A pivot over a real masked mart; a SQL-source chart through the browser; the assistant making a pivot from the quoted request (`AI-5-AC1`).
- `docs/core/PRODUCT.md` and `BACKLOG.md` were not touched (planner's).

**To check in a browser**

1. New chart > Data table > Raw rows on a 237-row mart: pick, reorder and remove columns; the preview shows the first rows; save. The tile pages to the last row and its footer says 237; the header sorts across pages and returns to page 1.
2. Edit that table: set one column to currency, hide one, swap two; save and reload; they stay. Add a link column holding `javascript:` and `https:` values: only the `https:` one is an anchor.
3. An image column: an `https` URL shows a small image; an `http` one shows its text.
4. A dashboard filter set before opening the table: the table, its total and its pages follow it; clear it and they follow that.
5. New chart > Pivot table: rows province and category, column a month (Group by month), value sum of visitors, Totals and subtotals. The grand total equals the sum of the table; a folded province keeps its subtotal; the corner cell is the grand total. Change Totals to None and Grand and check the total rows and columns go.
6. A pivot with five values over a big table: the tile says it was cut at 10,000 cells; the dashboard's "Group dates by" switch regroups a pivot that has a date field.
7. KPI > Compare with Previous period on a date column, Month: the number is the latest month, the change shows an amount, a percent, an arrow, green; Better when Down is good turns it red. A previous value of zero shows the amount only. Compare with Goal shows the percent of the goal and the distance.
8. In a records list (View records on a chart or a table), click a row: the one-record view lists every field; Next from row 50 lands on row 51 of page two; Previous goes back; Back returns to the page.
9. Export YAML (Dashboard menu) lists the new fields for a raw table, a pivot and a comparing KPI.
10. A user without `dashboard:write` cannot save any of it; a failing preview or page shows the sentence and reference, not database text.
11. Ask the assistant: "Show this as a pivot table with totals."

### BI-16 part A — fix handoff (developer, 2026-10-11, uncommitted, console only)

Rust not touched. Cited as "BI-16A review fix".

- **R1 (BLOCKER):** `pivot-table.tsx`. Cause: row-head cells used `bg-inherit` of a transparent `tr`, only the thead cells were opaque, nothing was stacked, and the table was squeezed. Now: table `w-max min-w-full` (scrolls sideways instead of squeezing); header cells `bg-card`, `z-20`, fixed `h-7` (second header row sticks at `top-7`); only the first row-field column sticks (`left-0 z-10`, opaque `bg-card`, or `bg-muted` on total rows, `min-w-28 max-w-56 truncate` with a `title`); the corner is `z-30`. Theme tokens only. Test pins opaque background and z-index on every sticky cell. *Not verified in a browser* (reasoned from the CSS).
- **R2 (BLOCKER):** cause: the preview route's spec has no `def`, so `TileBody` never saw column settings, pivot fields or the comparison (the `previewKey` already carried them). `src/lib/preview-spec.ts` `withPreviewDef` attaches the payload sent; `chart-builder.tsx` applies it to every preview result (mart and SQL-source paths). Tests (component layer, `table-tiles.test.tsx`): a spec without `def` shows the hidden column; with the attached payload it is hidden and the value is `Rp`; a KPI shows "75% of goal" only with the attached payload. The dialog itself is not rendered by any test.
- **R3 (SHOULD-FIX):** `pivot.ts` `labelFor`; `pivot-table.tsx` labels the grained field with `bucketLabel` (truncation grains). The tile does not carry which field was grained, so it is the first row/column field whose keys all have a date's shape, matching the server's choice (first date field). A part-of-date grain (day of week) on a pivot is not relabelled. Test: "Q3 2025".
- **R4 (SHOULD-FIX):** `cell-format.ts` number and percent now `en-US`; `kpi-compare.ts` signed amount/percent `en-US`; `kpi-card.tsx` value and change share `decimalsFor` and `formatNumber` from `chart-axis.ts`. Currency keeps `Rp`/`id-ID`. Tests updated for the convention (`cell-format.test.ts`, `kpi-compare.test.ts`: expectations changed from `1.234`/`+12,3%` to `1,234`/`+12.3%`, because the convention changed).
- **R5 (SHOULD-FIX):** collapsing needs two or more row fields. With one row field each group is a leaf, there is no subtotal row to stay in view, so there is nothing to fold; no change (a test pins it).

Counts: `bun run typecheck` clean; `bun run lint` 0 errors, 6 warnings (as before); `bun test src/features/dashboards src/lib`: 684 pass, 0 fail; full `bun run test`: see the final message.

## 7. Review (planner appends)

### BI-16 part A — findings from the product owner's QA (reviewer, 2026-10-11)

The reviewer did not re-run the suites (owner's instruction); the developer's counts are in the handoff, CI runs the rest.

QA passed: `BI-16A-AC1` (50 rows a page of 237, sort across pages), `AC2` after saving (currency, a hidden column and the order survive a reload), the pivot's grand total (278,955), the KPI comparison and the goal on the saved tile, `AC5` (record detail, "Record 6 of 56", Previous / Next).

- `BLOCKER` R1 — The pivot tile is unreadable: the sticky row-header column and the sticky header row have no opaque background and no reserved width, so row labels are drawn over the first value column ("Sumatera Barat" over "4.404"), the first row label over the column header, and the "Total" label over the first total. Give the sticky cells a background, a width and the right stacking; check with long labels, a narrow half-width tile and horizontal scroll, in light and dark themes.
- `BLOCKER` R2 — The builder preview does not show what is being edited for the new features: column settings of a raw table (currency, hidden column) and a KPI's comparison or goal appear only after saving. The preview must render through the same components and settings as the tile, and every new input must change it (`previewKey` and the preview renderer both).
- `SHOULD-FIX` R3 — A pivot column field grouped by a time grain shows the raw bucket start ("2025-07-01"); use `BI-9`'s bucket labels ("Q3 2025"), for row fields too.
- `SHOULD-FIX` R4 — Mixed number conventions on one tile: the KPI's value reads "11,299" and its change "-20.459 (-64,4%)"; table cells read "1.282" where charts read "1,282". New formatters must follow the convention the console already uses for chart and KPI numbers; only the currency format keeps its own (`Rp`).
- `SHOULD-FIX` R5 — The QA pivot (one row field, totals `all`) offers no collapse control. If collapsing needs two or more row fields, that is the answer and nothing changes; if a control should be there, fix it. Say which.

### BI-16 part A — closed (reviewer, 2026-10-11)

R1 to R5 are closed, confirmed by the product owner in a browser: the pivot reads cleanly with labelled quarters and a sticky first column; the builder preview follows column settings and a KPI's comparison or goal; numbers follow one convention; a two-level pivot collapses (R5: one row field has nothing to collapse, unchanged). No open `BLOCKER`.

Not verified by anyone: `BI-16A-AC6` as a user without `dashboard:write`; `AI-5-AC1` through the assistant; a raw table, pivot or comparing KPI on an embed or public link; a pivot over a masked table in a browser; the workspace-wide suites (CI).

Owed: the owner's answer on CSV export of a raw table (today: the 50 rows on screen, hidden columns included); decisions 1 to 7 on the feature page are planner defaults.

## 8. Addendum: whole-result CSV export of a raw table (owner's decision, 2026-10-11)

Decision 8 on the feature page. One task, T8.

| Decision | Choice |
| --- | --- |
| What is exported | Every row of the raw table's result, not the page on screen: the tile's visible columns (hidden ones left out) in their order, header = the column's label when set, else its name. The dashboard's active filters and the tile's sort apply. |
| Values | Raw, not display-formatted: numbers without separators or currency marks, dates as ISO text, so a spreadsheet can compute with them. |
| Limit | 100,000 rows. Beyond that the first 100,000 are exported and the console says the export was cut and at how many rows; nothing is cut silently. |
| Encoding | UTF-8 with a byte-order mark, CRLF line ends, RFC 4180 quoting. |
| Formula injection | A text cell that starts with `=`, `+`, `-`, `@`, tab or carriage return is prefixed with a single quote (OWASP CSV injection guidance). Numbers stay numbers. |
| Permissions | The statement goes through the same guard and role rewrite as the tile, so masks and row filters apply. Follow the existing download route of Query Studio (`tests/query_download.rs`) for the permission, the response headers and the audit entry; do not invent a second pattern. In `POLICY_TABLE` and `tests/route_auth.rs`. Not available on embeds and public links. |
| Where | The table tile's existing CSV action, for a raw table; grouped tables and other tiles keep today's export. The file is named after the tile. |

Verify, not assume: how Query Studio's download streams and caps, and reuse it; that an existing CSV helper (`src/lib/table-csv.ts`) already escapes formulas or not — one escaping rule, in one place; one real-engine run of the export statement with a filter and a sort, row count quoted.
