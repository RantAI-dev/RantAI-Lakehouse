# BI-10: bins, groups and hierarchies — Implementation Plan

**Status:** planner defaults, not started. Written by the planner (Claude
Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/bins-groups-hierarchies.md`. Specs:
`docs/core/specs/bi-10.md`, `docs/core/specs/ai-6.md` (the `BI-10` row).

**Base:** `feat/bi-phase-2`, after `BI-7` is committed (named in the
developer's brief). Read first: the `BI-6` and `BI-7` plans and handoffs;
`lakehouse-bi` `grain.rs` (a bin is to a number what a grain is to a date:
a bucket expression aliased to the grouping's name over the filtered
relation, with a click that becomes a range), `fields.rs` and `formula/`
(saved fields: a value group joins them), `shape.rs`, `builder.rs`; the
point-map path (`pointmap`, `geoheat`); `src/lib/bucket-click.ts`,
`src/features/dashboards/drill.tsx`, `chart-builder.tsx`.

**Verification:** as for `BI-6` (owner tests once at the end of the phase;
the developer runs the checks of section 6 of the `BI-6` plan, including
the integration files). Section 5 here is not reduced.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Number bins | Optional `bin` beside a grouping (the chart's `dimension`, `breakdown`, a table grouping, a pivot field) whose column kind is `Number`: `{ "mode": "auto" }`, `{ "mode": "count", "count": 10 | 50 | 100 }` or `{ "mode": "width", "width": <positive number> }`. A grouping carries a grain or a bin, never both. The result value is the bin's lower edge (a number); the tile carries the width so the console labels `500 – 1,000`. Lower edge included, upper excluded, the maximum in the last bin. `auto` and `count` read the column's minimum and maximum over the filtered, governed relation first and derive a round width (auto: about ten bins, widths of 1, 2 or 5 times a power of ten). Empty bins are not invented. Order: by bin, ascending, unless a sort says otherwise. |
| Click on a bin | Records: rows whose raw value falls in the bin (server-computed range). Filter: one numeric `between` filter, lower included, upper excluded (the existing exclusive flags). |
| Coordinate bins | `grid` on `pointmap` and `geoheat`: `{ "size": "auto" | 0.01 | 0.1 | 1 | 10 }` degrees. The server groups points by cell and returns one row per cell at the cell's centre with the aggregated measure and the point count; the existing point cap applies to cells. `auto` picks the largest listed size that yields at least 50 cells, else the smallest. |
| Value groups | A saved field of kind `group`, stored with the calculated fields of the table or SQL source (`console.bi_field`, a new kind column or discriminator; no Postgres migration): `{ "column": <a text column>, "groups": [ { "name": …, "values": [ … ] } … ], "rest": "keep" | "other" }`, at most 50 groups and 2,000 values in all, a value in one group only. Compiled by the server to a conditional over literals bound through the existing literal helper; never from text the user typed as an expression. It is a per-row text field: usable wherever a per-row calculated field is. Same routes, permissions, uniqueness and delete-in-use rule as calculated fields; the validate route checks a group definition too. |
| Hierarchies | A named, ordered list of 2 to 6 columns of one table or SQL source, in a new `console.bi_hierarchy` table created like the other BI tables. Routes under `/api/dashboard/hierarchies` (`GET`: `dashboard:read`; `POST`, `PUT`, `DELETE`: `dashboard:write`), in `POLICY_TABLE` and `route_auth`. The `fields` response lists a source's hierarchies. A column may be in several hierarchies. Deleting a hierarchy never breaks a chart: charts do not store it. |
| Drill down | Console-side and temporary. When a chart's `dimension` is a level of a hierarchy of its source and is not the last level, the click menu offers "Drill down to <next level>". The console then asks the preview route for the same definition with the dimension replaced by the next level and one more chart filter (`BI-6`) fixing the clicked value, and shows the result in the tile with a trail (All > Jawa Barat > Kota Bandung) whose entries climb back. Several steps stack their filters. Nothing is saved and the URL does not change. Not offered on a chart with a breakdown-only dimension change conflict, a table calculation, steps, `other`, a pivot or a raw table: plain reason in the menu. |
| Permissions | Bins, grids, groups and drill requests are ordinary tile statements through the same guard and role rewrite. The preview route already requires `dashboard:read`; a drill request can only add filters to a saved definition's shape, never read a different source — the server checks that the source of a preview with a `drillOf` chart id equals that chart's source for callers without `dashboard:write`. |
| Assistant | `bin` and `grid` in the `create_chart` / `update_chart` schemas. Tools: `create_value_group` (low-risk write), `list_hierarchies`, `create_hierarchy` (low-risk write), `delete_hierarchy` (approval). All named in `src/features/copilot/capabilities.ts`; `tool_schemas.json` regenerated, tool count assertion updated. |
| Export | `chart_def_fields` lists `bin` and `grid`. |

## 2. Tasks

- **T1 — `lakehouse-bi`:** `bin` (three modes, the min/max pre-read, labels' width on the result), its validation and click range; `grid` for the point kinds; unit tests per mode and edge (a single distinct value, negatives, all nulls, a zero-width range).
- **T2 — `lakehouse-bi` and API:** value groups as a field kind (store, compile, validate, routes, delete-in-use); hierarchies (table, routes, policy, `fields` listing).
- **T3 — API:** bins and grids through preview, dashboard, records, export, embed and public; the `drillOf` source check; assistant schemas, tools, fixture.
- **T4 — Console:** "Bin" control beside a number grouping; grid control on the point kinds; bin labels on axes, legends, tooltips and filter chips; bin click (records, filter).
- **T5 — Console:** value-group editor (pick values from the column's value list, name groups, rest as kept or "Other") opened from "New calculated field"'s neighbour "New value group"; hierarchy editor (ordered columns) under Data source; both listed with their own marks in pickers.
- **T6 — Console:** drill down in the click menu, the trail on the tile, climbing back.
- **T7 — Docs:** `CHANGELOG.md` `[Unreleased]`.

## 3. Out of scope (do not build)

A histogram chart type; hierarchies across joins; saved drill state; grids in metres or equal-area cells; groups over number or date columns (bins and grains cover those).

## 4. Things the developer must verify, not assume (not reduced)

Against the dev `ClickHouse` (26.8) and the real role rewrite, statements and results quoted:

- `BI-10-AC1`: `visitors` in `serving.mart_demo_map_points` binned by width 500: the counts add up to 237; `count: 10` yields at most ten bins covering minimum to maximum; `auto` gives round edges. The maximum value is in the last bin, not in a bin of its own.
- `BI-10-AC2`: a group over six provinces with `rest: "other"`: two rows adding up to 278,955; with `rest: "keep"` the other provinces keep their names. Values with quotes, backslashes and non-ASCII in a group definition come out only inside escaped literals (extend the injection property test).
- `BI-10-AC4`: grid at 1 degree: the cells' sums add up to 278,955 and each cell's centre lies inside its cell.
- The min/max pre-read and the binned statement both go through the role rewrite: with a row filter, the edges come from the filtered rows only; with a mask on the binned column, the engine's refusal is the same as for a plain chart.
- A drill request by a caller with only `dashboard:read` cannot be used to read another table or to drop the saved chart's own filters.
- Charts saved before this change produce the same SQL (the pinned exact-SQL tests).

## 5. Handoff (developer appends)

### Partial notes (developer, uncommitted)

- 2026-10-11 T1 done: `lakehouse-bi` new `bin.rs` (`BinSpec`, `Range`, `GridSpec`, `grid_points_sql`), `builder/binned.rs`, `Relation::Ranged`, `bin`/`breakdownBin`/`grid` on `ShapeFields`, `bin` on `Grouping`, pivot bin, validation in `shape.rs`, `FilteredSql.bins`. Mechanism deviation: the range (min/max/width) is a scalar subquery joined into the same statement instead of a separate pre-read (see handoff). Engine AC1/AC4 and rewrite run (`routes/bin_rewrite.rs`, 5 tests).
- 2026-10-11 T2 done: `lakehouse-bi` `groups.rs` (value group as a `bi_field` row, `level = "group"`, JSON in `formula`; no new column), `hierarchies.rs` + `console.bi_hierarchy` (DDL count 19 to 20 in the existing store test), `routes/hierarchies.rs`, `POLICY_TABLE` rows, `route_auth` test, `calc_fields` routes take `group`. T3 done: bins in tiles (`bins` on the payload via `annotate_table`/`annotate_bins`), preview bin reads, `routes/drill.rs` (`drillOf` builds the definition from the saved chart), records `binWidth`, export fields, assistant (`bin`, `breakdownBin`, `grid`, 4 tools, fixture regenerated, 75 tools). `cargo clippy -p lakehouse-bi -p lakehouse-api --all-targets -- -D warnings` clean; bi 378 pass, api lib 1619 pass. 
- 2026-10-11 T4, T5, T6, T7 written: `lib/chart-bins.ts`, `bin-picker.tsx`, `value-group-dialog.tsx`, `hierarchy-dialog.tsx`, `tile-drills.tsx`, builder/tile/page wiring, CHANGELOG. typecheck clean. Final checks and handoff next.

### Handoff (developer, 2026-10-11, uncommitted, on top of `3b3da21`)

T1 to T7 are built in the working tree of `feat/bi-phase-2-build` (`/home/hv/lakehouse-phase2-build`). Nothing is committed, pushed or stashed. The dev servers (:3100, :18081) were not touched and run the old build, so **nothing here was seen in a browser**. No Postgres migration. New ClickHouse table `console.bi_hierarchy`, created by the API code on first use.

**Files, by task**

- T1, `lakehouse-bi`: new `src/bin.rs` (`BinSpec`, `BinMode`, `Range`, `GridSpec`, `grid_points_sql`, `bin_records_predicate`), new `src/builder/binned.rs` (`Layer`, `binned`, `add_layer`, `reads`, `BinRead`, `needed_columns`), `builder.rs` (`Relation::Ranged`, `is_sql_source`, the bin layers in `rebuild_chart` (dimension, breakdown) and `groupings_sql`, `grid_sql`, `FilteredSql.bins`, `RecordsValue::Bin`), `tables.rs` (`pivot_sql` takes the bin beside the grain, `pivot_bin_field`), `shape.rs` (`bin`, `breakdownBin`, `grid` on `ShapeFields`, `bin` on `Grouping`, `check_bins_kind`, `check_bins_against_relation`), `store.rs` (pivot needs a number field), `lib.rs`.
- T2: new `src/groups.rs` (value group definition, validation, compile), `fields.rs` (a group is a field with `level = "group"`, compiled in `prepare_refs`, left out of the formula scope), new `src/hierarchies.rs`, `store.rs` (`console.bi_hierarchy` DDL in `ensure_field_tables`), API `routes/hierarchies.rs`, `routes/calc_fields.rs` (`group` on create, update, validate), `routes/mod.rs`, `policy.rs` (4 rows), `fields` responses list `hierarchies` (an optional lookup: a failure gives an empty list and a logged reference), `tests/route_auth.rs` (+1).
- T3, API: `routes/support.rs` (`annotate_bins` reads each binned column's width, min, max through the tile's own role rewrite and puts `bins` on the tile; `preview_bin_reads` for the builder's preview), `routes/dashboard.rs` (records `binWidth`, preview `drillOf`, export fields `bin`, `breakdownBin`, `grid`), new `routes/drill.rs`, assistant: `routes/ai/registry.rs` (`bin`, `breakdownBin`, `grid`, `groupings[].bin`; tools `create_value_group`, `list_hierarchies`, `create_hierarchy`, `delete_hierarchy`), `tools/calc_fields.rs` + `tools/mod.rs` (one guard arm, `handles`), `prompt.rs`, `tests/fixtures/tool_schemas.json` (regenerated, 374 lines added, 71 to 75 tools), `src/features/copilot/capabilities.ts`. Test modules: `routes/bin_rewrite.rs` (7 tests, declared last in `dashboard.rs`).
- T4 to T6, console: new `lib/chart-bins.ts`, `lib/value-groups.ts`, `lib/hierarchies.ts` (+ tests), `features/dashboards/bin-picker.tsx`, `value-group-dialog.tsx`, `hierarchy-dialog.tsx`, `tile-drills.tsx` (+ tests), `services/contracts|clients/hierarchies.ts`, `services/index.ts`, `contracts|clients/calc-fields.ts` (`createGroup`, `updateGroup`, `isValueGroup`, `groupDefOf`), `lib/chart-shape.ts` (bin and grid in the draft and the payload), `chart-builder.tsx` (number columns offered as dimension and breakdown for the kinds that bin; pickers; grid; chips for groups and hierarchies; dialogs), `chart-shape-section.tsx` (bin per grouping), `tile-body.tsx` (labels), `lib/chart-click.ts` (click on a bin), `records.ts` (`binWidth`), `drill.tsx` (menu line "Drill down to <level>" or the plain reason), `dashboard-page.tsx` (bin filter, drill state, trail), `lib/geo-points.ts` (a grid cell says how many points it holds), `lib/page-context-summary.ts` (the assistant is told a tile is binned), `features/copilot/chart-draft-card.tsx`, `tile-dialogs.tsx` (`bins` on `Rows`). `previewKey` needed no change: its `shape` input already serialises the whole `ShapeDraft`, which now holds `bin`, `breakdownBin` and `grid`.
- T7: `CHANGELOG.md`.

**Mechanism deviation (planner to accept or reverse)**: the plan says `auto` and `count` read the column's minimum and maximum "first". I read them in the **same statement** (a one-row subquery joined in, over the very same filtered relation text, `Relation::Ranged`), not in a separate pre-read. Why: the edges can never come from different rows than the bins hold; the role rewrite sees every read as a plain table read and substitutes the governed projection at both (section 4, below); and no call site (dashboard, preview, embed, public, export, records) needs a second asynchronous read. The tile still needs the width to label `500 – 1,000`, so the builder also returns a `(width, min, max)` statement per binned column (`FilteredSql.bins`), which the API runs through the same rewrite (`bins` on the tile). The last bin includes the maximum for every mode, also a set width; the records list of a clicked bin closes its upper edge when that edge is the maximum of the same filtered rows.

**Commands run, with counts** (rust from `/home/hv/lakehouse-phase2-build/rust`, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=4`, one command at a time, `-p` scoped, both `lib.rs` touched before the final clippy)

- `cargo fmt -p lakehouse-bi -p lakehouse-api -- --check`: 0 diffs (I ran `cargo fmt` on those two crates; every file it touched is mine).
- `cargo clippy -p lakehouse-bi -p lakehouse-api --all-targets -- -D warnings`: clean (it found: two functions over 100 lines, a complex type; fixed by extracting `grid_sql`, `group_column`, `ensure_field_tables`, `calc_fields::handles`, `PivotInner`).
- `cargo test -p lakehouse-bi`: 378 lib pass, 1 ignored, `specs_match_typescript` 1 pass. `cargo test -p lakehouse-api --lib`: 1619 pass, 1 ignored (the fixture writer).
- Each alone: `route_auth` 35 pass (34 + the hierarchies test), `sec11_guard` 4, `query_author_exception` 6, `dashboard_export` 6, `embed_tokens` 21, `ai_tool` 9. Not run (not on the brief's list): `security_regressions`, `upstream_errors`.
- `bun run typecheck` clean; `bun run lint` 0 errors, the same 6 warnings; `bun run test`: 1335 pass, 1 skip, 0 fail, 149 files (109 s). New test files: `chart-bins` 8, `value-groups` 5, `hierarchies` 5, `bin-picker` 3, `value-group-dialog` 3, `hierarchy-dialog` 3, `tile-drills` 4, `use-tile-drills` 4; added cases in `chart-click`, `chart-shape` (4), `geo-points`, `page-context-summary`, `table-tiles`.
- Engine: `docker exec lakehouse-clickhouse-1 clickhouse-client` for every statement below (the tests print them with `BIN_STATEMENTS_OUT` and `BIN_REWRITE_OUT`); read-only on `serving.*`; one scratch database `scratch_bi10` (5 tiny tables) used for the edge cases, **dropped at the end** (see below).

**Existing tests changed** (none weakened, no assertion lost): `store::tests::ensure_bi_table_runs_ddl_at_most_once_per_process` 19 to 20 DDL statements (the new table); `registry::tests::tool_schemas_has_seventy_one_entries` renamed `..._seventy_five_entries` and expects 75; `create_chart_and_update_chart_take_the_chart_builder_fields` also lists `bin`, `breakdownBin`, `grid`; three `tables.rs` pivot tests pass `(grain, None)` for the new argument; `shape.rs` tests that build a `Grouping` gain `bin: None`; `chart_def_fields` test gains bin and grid cases; the fixture `tool_schemas.json` regenerated and pinned by `tool_schemas_snapshot_is_byte_identical`.

**Section 4, on the dev `ClickHouse` 26.8.9, real `sql_rewrite::enforce`** (`routes/bin_rewrite.rs`; policies: `row-filter` = row filter `provinsi = 'Dki Jakarta'` and mask `place`; `mask-visitors` = the same row filter and a mask on `visitors`; each test asserts the table's name appears as many times in the rewritten statement as in the original, with the row filter, and the mask where there is one, at every read)

1. `BI-10-AC1`. `visitors` of `serving.mart_demo_map_points` (237 rows, min 44, max 2,377), count of rows, width 500: bins `0 48`, `500 54`, `1000 55`, `1500 41`, `2000 39` = 237. The statement: `SELECT visitors, count() AS lat FROM (SELECT round(least(floor(toFloat64(visitors) / __bw_visitors), if(__bmx_visitors > __bmn_visitors, ceil(__bmx_visitors / __bw_visitors) - 1, floor(__bmx_visitors / __bw_visitors))) * __bw_visitors, 10) AS visitors, lat FROM (SELECT * FROM serving.mart_demo_map_points CROSS JOIN (SELECT w AS __bw_visitors, mn AS __bmn_visitors, mx AS __bmx_visitors FROM (SELECT mn, mx, 500 AS w FROM (SELECT min(toFloat64(visitors)) AS mn, max(toFloat64(visitors)) AS mx FROM serving.mart_demo_map_points))) AS rng) AS ranged) AS bkt GROUP BY visitors ORDER BY visitors LIMIT 100`. The maximum 2,377 is in the last bin `2000`, not alone. With a width of 2,377 (the maximum itself a multiple of the width) all 237 rows are in bin 0, and the records predicate of that last bin returned 237 rows. `count: 10` gave width 500 (5 bins, at most ten, covering 44 to 2,377); `auto` the same (round edges 0, 500, ..., 2000); `count: 100` width 50, 48 bins (0 to 2,350). The width statement returned `500 44 2377`. Clicked bin 2000 to 2500: 39 rows, the bar's count.
2. `BI-10-AC2`. A group "Jawa" over Jawa Barat, Jawa Tengah, Jawa Timur, Banten, Dki Jakarta, "DI Yogyakarta" with `rest: "other"`: `Other 153795`, `Jawa 125160` = 278,955. (The demo data spells it "Daerah Istimewa Yogyakarta" and has no Banten, so only four of the six names match; the checklist below uses the real spellings.) With `rest: "keep"`: `Jawa 125160`, then the other provinces by their own names (Sulawesi Selatan 12,799, Sumatera Barat 11,593, ...). The compiled expression is `multiIf(toString(provinsi) IN ('Jawa Barat', ...), 'Jawa', 'Other')`. Injection: the property test in `groups.rs` builds 18 definitions from the hostile values `O'Brien`, `x') OR 1=1 --`, a backslash, a tab, non-ASCII, `'; DROP TABLE t; --` crossed with three hostile group names, strips every well-formed literal and asserts that only the fixed skeleton is left; all 18 statements were also run on the engine against a table holding those values (18 ran, each changed at most the one value it names, and `system.tables` shows no table `t`). Under the mask on `provinsi` (the group reads the masked text) every row falls to the rest: `Other 63696` (Office rows only, the role's second filter).
3. `BI-10-AC4`. Grid size 1 over the demo points: 30 cells, `sum(visitors)` 278,955, 237 points; every cell's latitude equals `floor(lat / size) * size + size / 2` of its own cell (0 mismatches). `auto` picked 0.1: 73 cells (1 degree gave 30, under 50), same sum and point count. Under the row filter: one cell, `-6.5 106.5 19400 15 1`.
4. The min/max read and the binned statement both go through the rewrite. Row filter: the tile for `auto` returned `0 14`, `500 21`, `1000 12`, `1500 8`, `2000 15` (70 rows) and the width statement `500 47 2356`, against `44 / 2377 / 237` unrestricted: the edges come from the filtered rows only. Mask on the binned column: the tile and the width statement are refused by the engine (`Code: 72 ... Cannot read floating point value here: **`), a plain chart over the same masked number is refused too (`Code: 43 ... Illegal type String of argument for aggregate function sum`); a grid over the masked column the same (`Code: 43`). No value is returned in either case.
5. Drill. `routes/drill.rs` builds the definition **on the server from the saved chart**: the body may carry only `drillOf`, `to`, `fix` and the dashboard's `filters` (`deny_unknown_fields`: a `mart` is refused), the source, measures, breakdown and the chart's own filters are the saved chart's, `to` and every fixed column must be consecutive levels of one hierarchy of the chart's source that holds the chart's dimension, and the only filters added are equalities on the fixed columns and the dashboard's. Unit tests (5) cover each refusal. On the engine a drilled level (`kab_kota`, saved filter `category IN ('Office','Retail')`, click `provinsi IN ('Jawa Barat')`) under the Jakarta row-filter role returns **no rows**, although Jawa Barat has 11 rows unrestricted: the row filter holds and the click cannot widen it. I did not call the preview route over HTTP (no server of my own); the route test of this path is the builder and rewrite statements above, plus `drill::tests`.
6. Charts saved before this change: no existing expectation was edited except the pivot call signature; `builder::tests` (the BI-6 pinned statements, grained, filtered, SQL source, staged, BI-7 joined) all pass; a new test shows a chart without a bin builds `SELECT stored` (no range) and carries no `bins`.
7. Edge cases of the width on the engine (scratch tables): one distinct value (width 1, bin 7), negatives (`-1500 1`, `-500 1`, `0 2`, `500 1`), all NULL (one NULL group, no invented bin), fractions 0.01 to 0.5 (width 0.05, edges 0, 0.35, 0.45), two values 1 and 2 (width 0.1; 2 is in the closed last bin `1.9`).

**Plan versus code, decisions where the plan was silent**

1. Mechanism: see above (one statement, not a pre-read).
2. A pivot bins its **first number row or column field** (one bin per pivot, as it has one grain); the bin travels as the chart's `bin`.
3. A grouped table bins per grouping (`groupings[].bin`); top-level `bin` is refused beside groupings.
4. "Automatic and counted bins on a SQL source": refused at save with a plain sentence ("choose a width"); a width works there. A fixed-width bin on a SQL source reads the source twice.
5. Value groups are **not usable inside a formula** (the formula language reads columns and formula fields only); they are usable as dimension, breakdown, filter and table column, and refused as a measure. The plan said "usable wherever a per-row calculated field is"; this is the one place they are not.
6. Drill: the preview request does not carry a definition (plan: "the same definition with the dimension replaced"); the server builds it (stronger than the plan's `drillOf` source check, which would have trusted the rest of a client-sent body). The drill is offered on bar, hbar, line, area, stacked, pie, rose, funnel and treemap; not on a pivot, table, KPI, map, or a chart with a grain, bins, steps, "Other" or groupings (plain reason in the menu, shown only for a column that is in a hierarchy). A drilled tile follows the dashboard's filters (re-requested on a filter change) and its records list carries the clicked values.
7. The value-group routes take `group` beside `formula`; a group stays a group on update. Stored in `console.bi_field` with `level = "group"` and the definition JSON in `formula` (no column added).
8. A grid has no label column: its marks say how many points the cell holds (`cell_points`); the cell size is returned as `cell_size`.
9. `run_tool` was at clippy's line limit: the four new tools share the existing calculated-field arm through `calc_fields::handles`.
10. The builder lists number columns as dimension choices (and breakdown) for the kinds that bin, and starts an auto bin (a width on a SQL source) when one is picked; "No bins" groups by each distinct number as before.

**Not verified**

- Anything in a browser (the pickers, bin labels on axes, tooltips and legends, the bin click menu, the two dialogs, the drill trail), embeds and public links with bins, a drilled tile in an embed (not offered there), `bins` on the preview of a SQL source (the console falls back to the chart's fixed width).
- The assistant through the model (`AI-6-AC1`): schemas, fixture, registry tests and the draft card only.
- The preview route over HTTP with `drillOf`; the hierarchy and value-group routes against a real Postgres-backed session beyond `route_auth` (both ways for hierarchies are asserted; the handlers were not called with a live ClickHouse).
- A big table (the range subquery makes a binned statement read the table twice per bin, or three times with "Other"): not timed.
- `security_regressions`, `upstream_errors`, workspace-wide clippy and test, `docker compose`, the gates (CI).

**Browser checklist** ("Map demo", `/dashboards/b_980236ed`; restart the API and console on this tree first; demo table `mart_demo_map_points`: 237 rows, `visitors` 278,955, min 44, max 2,377, 26 provinces)

1. **New chart**, **Bar**, data source `mart_demo_map_points`. Dimension: pick `visitors` (listed among the columns). A "Group in" control appears with "Automatic (about 10 bins)". Measure `lat` with aggregation `count`. Expect 5 bars `0 – 500` 48, `500 – 1,000` 54, `1,000 – 1,500` 55, `1,500 – 2,000` 41, `2,000 – 2,500` 39 (237).
2. Change "Group in" to **10 bins**: the same 5 bars. **100 bins**: 48 bars of width 50 (`0 – 50` 3, `50 – 100` 3, `100 – 150` 4 ...). **A width of my own**, type 500: the five bars again; empty or 0 lists "Dimension bin width" as still needed. **No bins**: one bar per distinct number.
3. Under **Filters** add `provinsi` is Dki Jakarta: the automatic bins redraw from Jakarta's 70 rows only (`0 – 500` 14, `500 – 1,000` 21, `1,000 – 1,500` 12, `1,500 – 2,000` 8, `2,000 – 2,500` 15).
4. Create the chart. Click the `2,000 – 2,500` bar: the menu title reads `visitors: 2,000 – 2,500`. **View records** lists 39 rows (no filter) with `visitors` from 2,000 to 2,377. **Filter dashboard by this**: a chip for `visitors` between 2,000 and 2,500, the tile narrows to that bin; click the same bar again: the filter goes. Click `500 – 1,000`: **View records** 54.
5. Set a **Breakdown** of `lon` with "Group in" **A width of my own** 10: the legend reads ranges; a grouped **Data table** with `visitors` (auto) and `category` groups reads ranges in the first column. A **Pivot** with row field `visitors` shows a "Group visitors in" control.
6. **Map — points**, lat `lat`, lon `lon`, measure `visitors` sum: under "Marks" choose **Grid cells of 1°**: 30 marks, tooltip values add up to 278,955 and the point counts to 237 ("15 points" for the Jakarta cell at 19,400). **Grid, automatic size**: 73 marks. **One mark per point** brings back the points.
7. **New value group** (next to New calculated field): name `region_group`, column `provinsi`; in Group 1 rename it `Jawa` and tick Jawa Barat, Jawa Tengah, Jawa Timur, Dki Jakarta, Daerah Istimewa Yogyakarta; "The other values" **Call them Other**; Create. A bar with dimension `region_group` (marked "group"), measure `visitors` sum: two bars, `Jawa` 133,381 and `Other` 145,574 (278,955). With **Keep their own names**: `Jawa` then Sulawesi Selatan 12,799, Sumatera Barat 11,593 ... Ticking a value that is in another group is disabled and names its group. Delete the group while a chart uses it: refused naming the chart.
8. **New hierarchy**: name `Geo`, levels `provinsi`, `kab_kota`, `place`; Create (the chip `> Geo` appears). Add a bar `provinsi` by `visitors` (sum) to the dashboard. Click **Jawa Barat** (13,836): the menu offers **Drill down to kab_kota**; choose it: the tile now shows `Kota Bogor` 8,736 and `Kota Bandung` 5,100, with the trail **All > Jawa Barat**. Click **Kota Bogor**, **Drill down to place**: seven places (`Bogor #7` 2,035 first), trail **All > Jawa Barat > Kota Bogor**; clicking a place offers no drill (last level, the menu says so). **View records** on a drilled bar lists only that province and city. Click **Jawa Barat** in the trail: back to the cities; **All**: the original bar chart. Reload: the drill is gone (nothing saved) and the address did not change. A dashboard filter added while drilled narrows the drilled level too.
9. A chart with a grain, bins, steps or "Other" whose column is in the hierarchy: clicking a value shows "Cannot drill down: <plain reason>" and nothing else changes.
10. As a user without `dashboard:write`: saving a value group or a hierarchy fails with a plain message ("You do not have permission..."), and the existing groups and hierarchies still work for a reader (`BI-10-AC5`).
11. As a role with a row filter (Governance): the same bin chart shows edges from that role's rows only, a mask on `visitors` shows the tile's fixed error message, and a drilled level never shows another role's rows.
12. Ask the assistant: "visitors per place in bins of 500". It should call `create_chart` with `bin: {mode: "width", width: 500}` (`AI-6-AC1`; not run by me).

## 6. Review (planner appends)

### BI-10 — planner answers (reviewer, 2026-10-11)

Built in a separate worktree while the dev console was frozen for a product demo, so the reviewer's smoke check on a rebuilt dev API has **not** been done for this item yet; it is owed before the end-of-phase QA. No per-item browser QA and no reviewer re-run of the suites (owner's instruction for phase 2).

Planner answers to the handoff: deviations 1 to 4 are accepted (min and max read inside the statement; the drill definition built on the server from the saved chart, which is stricter than the plan; value groups not usable inside a formula or as a measure; storage and bin scope as built). Deviation 5: the acceptance wording follows the data's own spellings.

Owed: the smoke check; the drill preview route has never been called over HTTP; `security_regressions` and `upstream_errors` at the end of the phase.

### BI-10 — smoke check, done after the demo (reviewer, 2026-10-11)

On the dev API rebuilt from `845ae01`, against the real engine:

- Bins of width 500 over `visitors`: five bins whose counts add up to 237 (48, 54, 55, 41, 39), with the bin metadata (`width`, `min` 44, `max` 2377) on the result.
- A hierarchy province > city > place created over HTTP; the drill preview, called over HTTP for the first time, returns for Jawa Barat the two cities and totals of a direct query (Kota Bogor 8,736, Kota Bandung 5,100), then seven places adding up to 8,736 for Kota Bogor. Skipping a level, and drilling a chart with an "Other" group, are refused with plain sentences.
- The demo dashboard's existing tiles return no error.

- `SHOULD-FIX` R1 — A binned chart saved without a `sort` comes back ordered by its measure, descending (the bar default), not by bin ascending as the plan's decision table says. The server default for a binned dimension, with no `sort` and no explicit `order`, is bin order. To be fixed at the start of the next item.

Not looked at by anyone: the Bin control, the value-group editor, the hierarchy editor, the drill menu and trail on a tile, the grid map.
