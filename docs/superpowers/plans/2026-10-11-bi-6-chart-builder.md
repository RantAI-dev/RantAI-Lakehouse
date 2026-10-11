# BI-6: a more capable chart builder — Implementation Plan

**Status:** planner defaults, not started. Written by the planner (Claude
Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/chart-builder-questions.md`. Specs:
`docs/core/specs/bi-6.md`, `docs/core/specs/ai-6.md` (the `BI-6` rows).

**Base:** `feat/bi-phase-2` at `a34db4d` (phase 1 complete). Read first: the
handoff and review sections of the four phase 1 plans in
`docs/superpowers/plans/2026-10-1*-bi-*.md` (same code, same pitfalls);
`lakehouse-bi` `builder.rs` (`QueryBuilder::build`, `Relation`, `rebuild`,
`filter_predicates`), `grain.rs`, `fields.rs`, `formula/`, `staged.rs`,
`tables.rs`, `store.rs` (`ChartInput`, `limit_and_order`, validation);
`routes/dashboard.rs` (records, preview, export); `src/features/dashboards/
chart-builder.tsx`, `src/lib/preview-key.ts`, `src/lib/preview-spec.ts`,
`filters/` (the filter editors to reuse).

**Verification on this slice (owner's instruction, 2026-10-11):** the owner
tests by hand; the reviewer does not re-run suites. The developer runs the
checks in section 6, which include the integration test files of every
route touched (phase 1's CI went red on an embed test nobody had run).

**Two parts, two handoffs.** Part 1 is T1 to T5. Stop after it.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Compatibility | Every chart saved before reads and renders exactly as before. All new fields are optional; absent means today's behaviour. No migration. |
| Measures with their own aggregation | `measures` accepts, beside a name, an object `{ "column": …, "aggregate": … }` (the shape a pivot's `values` already uses). A plain name takes the chart's `aggregate`, as today. A calculated field of aggregate or table level ignores the aggregation, as today. The result column is named so two aggregations of one column do not collide, and the console shows a readable label (`avg of stay`). |
| A chart's own filters | Optional `filters` on the definition: a list of the `FilterDef` of `BI-18` part A (same validation, same limits), on columns of the chart's source, including per-row calculated fields. They are always applied, before and in addition to the dashboard's filters; a dashboard filter cannot remove or widen them. They are returned on the tile so the console can list them. Records and CSV export of the tile apply them too. |
| Sort | Optional `sort`: `{ "by": <a grouping or a measure of this chart>, "dir": "asc" | "desc" }`. Absent means today's `order`. |
| Top or bottom N, with "Other" | `limit` with `sort` keeps the first N groups. Optional `other: true` adds one group labelled `Other` computed from the underlying rows that fall outside the kept groups (one more pass of the same aggregation over them), not from the hidden results. With a breakdown, N applies to the dimension and `Other` is computed per series. Not offered with a time grain, a table calculation, a pivot or a raw table: refused with a plain sentence. `Other` carries a flag on the tile so the console offers no value drill on it and never confuses it with a real value named "Other". |
| Groupings | A grouped `table` takes `groupings`: 1 to 5 columns (or per-row fields), each optionally with a `BI-9` grain. Visual charts keep `dimension` and `breakdown`; a pivot keeps `rows` and `columns`. |
| Steps (part 2) | Optional `steps`, at most two, each applied to the result of the one before: `{ "filter": [conditions on that result's columns: a grouping or a measure, with the comparison operators and `between`] }` or `{ "summarise": { "by": [a subset of that result's groupings], "measures": [{ "column": <a measure of that result>, "aggregate": … }] } }`. Values are bound through the existing literal helpers; names are validated against the previous step's output. Compiled as nested statements. |
| Preview rows (part 2) | The preview route returns up to 2,000 rows of the final result with `truncated` when cut; the builder shows them in a table beside the chart. A saved chart's own limit is not changed by this. |
| Rebuild | A chart that uses any of these is always rebuilt at read time from its definition, like a grained chart. |
| Permissions | Every statement shape goes through the same guard and role rewrite as a tile today. |
| Assistant | `create_chart` / `update_chart` schemas gain the new fields; `tool_schemas.json` regenerated. No new tool is expected; if one is added, it must also be named in `src/features/copilot/capabilities.ts` (the test there fails otherwise). |
| Export | `chart_def_fields` (YAML export) lists the new fields. |

## 2. Tasks, part 1

- **T1 — `lakehouse-bi`:** the definition fields and their validation; per-measure aggregation; chart filters merged with dashboard filters; `sort`; top/bottom N with `Other`; table `groupings`. Unit tests per shape and per refusal.
- **T2 — API:** preview, dashboard payload (filters and the `Other` flag on the tile), records and CSV export honour the chart's filters; embed and public paths; YAML export; assistant schemas and fixture.
- **T3 — Console builder:** measures with their own aggregation (add, remove, reorder), a "Filters" section reusing the `filters/` editors, sort control, top/bottom N with the "Other" switch, groupings for a table. Every new input reaches `previewKey` and `previewSpec`.
- **T4 — Console tiles:** the chart's own filters listed on the tile (compact, not removable), readable measure labels in legend, axis and tooltip, `Other` drawn last and not clickable for drill.
- **T5 — Docs:** `CHANGELOG.md` `[Unreleased]`.

## 3. Tasks, part 2 (do not start until told)

- **T6 — Steps** in `lakehouse-bi` and the API.
- **T7 — Preview rows** (route and builder).
- **T8 — Console:** the steps editor; docs.

## 4. Out of scope (do not build)

Drag and drop; joins; bins, groups, hierarchies; parameters; two sources on one chart; more than two groupings on a visual chart; "Other" on time-grained charts.

## 5. Things the developer must verify, not assume (not reduced)

Against the dev `ClickHouse` (26.8), statements and results quoted:

- `BI-6-AC4`: top 5 provinces by `sum(visitors)` with `Other` on `serving.mart_demo_map_points` — the groups add up to 278,955; with `avg(visitors)` the `Other` value equals a direct average over the remaining rows.
- A chart filter on a column that the dashboard also filters: both apply (intersection), in the plain form, with a breakdown, with a grain, with a calculated field, and on a SQL source.
- Two aggregations of the same column in one chart do not collide in the result.
- The role rewrite on the new shapes (the `Other` pass, table groupings, and in part 2 the nested steps): with the real `enforce` and a masked column plus a row filter, every read of the table is the masked and filtered projection. If the rewrite cannot see into a shape, stop and report.
- A real value literally named `Other` in the data is kept apart from the computed group.
- That charts saved before this change produce byte-identical SQL where they did before (pick three stored definitions from the tests).

## 6. Checks

Per task: `cargo fmt --check`, `cargo clippy -p <crate> --all-targets -- -D warnings`, the unit tests of the files touched; `bun run typecheck`, `bun run lint`, the bun test files touched. At the end of each part: `cargo test -p lakehouse-bi`; `cargo test -p lakehouse-api --lib`; each of these integration files alone: `route_auth`, `sec11_guard`, `security_regressions`, `embed_tokens`, `query_author_exception`, `dashboard_export`, `ai_tool`, `upstream_errors`; one full `bun run test`.

## 7. Handoff (developer appends one entry per part)

### Partial notes (developer, uncommitted)

- 2026-10-11 T1 done (not committed): `lakehouse-bi` new `shape.rs` (`MeasureList`, `ShapeFields`, `SortSpec`, `Grouping`, save-time checks), `builder.rs` (`Measure`, `Sort`, per-measure aliases, own filters merged in `report_over`, `stored_sql`, `groupings_sql`, `sort_of`), new `builder/ranked.rs` (the top-N with `Other` and the breakdown-sorted statements), `store.rs` (`ChartInput.measures` is a `MeasureList`; `shape` flattened; `finish_shaping`, `spec_from_groupings_input`), `fields.rs` (own filters and groupings are field slots), `grain.rs` (`validate_column`). `cargo test -p lakehouse-bi`: 319 lib + 1 pass; `cargo clippy -p lakehouse-bi --all-targets -- -D warnings` clean; `cargo fmt --check -p lakehouse-bi` clean. Real-engine runs of the new statements: see the handoff. Next: T2 (API).
- 2026-10-11 T2 done (not committed, Rust side; the console side is T3 and T4): `routes/dashboard.rs` (records route takes `chart` and carries the tile's own filters, table export too; YAML export lists the new fields; `GET /api/dashboard` and the embed path already carry `def`, whose `filters` the console lists), `routes/ai/registry.rs` (`measures` accepts `{column, aggregate}`; `filters`, `sort`, `other`, `groupings` on `create_chart` and `update_chart`), `tests/fixtures/tool_schemas.json` regenerated, new `routes/chart_shapes_rewrite.rs` (role rewrite on the new shapes). Also fixed during the real-engine runs: a plain measure beside another aggregation of its column was an engine error 184 (alias read as the aggregate), now `__m<i>` names inside and renamed outside (`select_lists`). `cargo test -p lakehouse-bi`: 320 lib + 1; `records_pages` 17 pass.
- 2026-10-11 T3 and T4 done (not committed): `lib/chart-shape.ts` (+ test, 14), `features/dashboards/chart-shape-section.tsx` (measure rows, chart filters, groupings; test 12), `chart-builder.tsx` (measure rows replace `measure`/`measure2`/`measure3`, one sort control, Other switch, filters section, groupings for a grouped table, every input in `previewKey` through `shape`), `chart-option.ts` (several measures on a bar or line, labels), `tile-body.tsx`, `chart-click.ts` (no drill on Other), `filters/own-filters-marker.tsx` (tile, embed, public), `records.ts` (`chart`), copilot context and chart draft card. `bun run typecheck` clean; `bun run lint` 0 errors, same 6 warnings. T5 (CHANGELOG) written.

### Handoff — part 1 (developer, 2026-10-11, uncommitted)

Tasks T1 to T5 are built in the working tree of `feat/bi-phase-2` (base `4609d3f`, docs only on top of `f4a64ca`). Nothing is committed, pushed or stashed. Part 2 (T6 to T8) is not started. The dev servers on :3100 and :18081 were not restarted: they still run the old build, so nothing below was seen in a browser (see "Not verified").

**Files changed, by task**

- T1, `lakehouse-bi`:
  - New `src/shape.rs`: `MeasureList` (column names as a slice, an optional aggregate beside each; serde reads a name or `{column, aggregate}` and writes it back the same), `ShapeFields` (`filters`, `sort`, `other`, `groupings`, flattened into `ChartInput`), `resolve_measures` (the result name `avg_stay`; a calculated aggregate or table field keeps its own name), the save-time checks (`check_kind_fields`, `check_against_relation`).
  - New `src/builder/ranked.rs`: the top-N with `Other` statement, and the breakdown-sorted-by-a-measure statement.
  - `src/builder.rs`: `Measure`, `Sort`, own filters merged ahead of the year and dashboard filters in `report_over` (`with_own_filters`; one that cannot apply fails the tile, never dropped), `stored_sql` (what a chart that uses the new fields is saved with), `groupings_sql` (nested `Relation::Bucketed`, one per grained grouping), `select_lists` (the `__m<i>` renaming), sort in `build`, `order_words`.
  - `src/store.rs`: `ChartInput.measures` is a `MeasureList`; `ChartInput.shape`; `uses_shaping`, `effective_order`; `finish_shaping`; `spec_from_groupings_input`; validation hooks. `src/fields.rs` (own filters and groupings are field slots), `src/grain.rs` (`Grain::validate_column`), `src/builder/staged.rs` (result names), `src/lib.rs`.
- T2, `lakehouse-api`: `routes/dashboard.rs` (records route takes `chart` and carries the tile's own filters ahead of the dashboard's, 422 when one no longer applies, 400 when the tile reads other data, 404 when unknown; whole-result CSV export carries them; YAML export lists the new fields), `routes/ai/registry.rs` (`measures` accepts `{column, aggregate}`; `filters`, `sort`, `other`, `groupings` on `create_chart` and `update_chart`), `tests/fixtures/tool_schemas.json` regenerated (reviewed schema change, 334 lines added), new `routes/chart_shapes_rewrite.rs` (declared at the end of `dashboard.rs`, because `routes/mod.rs` may have no non-test item after a `#[cfg(test)]` module: `tests/sec11_guard.rs` said so), `routes/embed.rs` (a test), `routes/calc_fields.rs` (a test helper's call).
- T3 and T4, console: `src/lib/chart-shape.ts`, `src/features/dashboards/chart-shape-section.tsx` (measure rows with add, remove, reorder and an aggregate each; the chart's own filters, inline, with the dashboard's editors; a table's groupings), `chart-builder.tsx` (one Sort control: the three old orders, then each grouping and measure both ways; the "Other" switch; every new input in `previewKey` through `shape`, and the preview renders through `withPreviewDef` with the payload, so the labels and the filters follow), `chart-option.ts` (one series per measure on a bar, hbar, line and area; "avg of stay" in legend, axis and tooltip), `tile-body.tsx` (labels; the `__other` flag is not a column; a real value called "Other" is kept apart: the computed group is drawn "Other (rest)" only when both exist), `chart-click.ts` (no drill on the computed group), `filters/own-filters-marker.tsx` (the filters listed on the tile, in the dashboard, the embed and the public view), `records.ts` + `dashboard-page.tsx` + `drill.tsx` (the `chart` parameter), `features/copilot/chart-draft-card.tsx` and `lib/page-context-summary.ts` (assistant: "Edit in builder" keeps the new fields; the dashboard's context says which tiles are filtered).
- T5: `CHANGELOG.md` `[Unreleased]`.

**Commands run, with counts** (all from `/home/hv/lakehouse-uiux/rust` with `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2`, one at a time, `-p` scoped; crate `lib.rs` touched before the clippy and test runs)

- `cargo fmt --check -p lakehouse-api -p lakehouse-bi`: clean (ran `cargo fmt` on these two crates only; the files it touched are all files with changes of mine).
- `cargo clippy -p lakehouse-bi -p lakehouse-api --all-targets -- -D warnings`: clean (one run, both `lib.rs` touched first).
- `cargo test -p lakehouse-bi`: 320 lib tests pass, 1 ignored, plus `specs_match_typescript` 1 pass, doc-tests 0.
- `cargo test -p lakehouse-api --lib`: 1597 pass, 1 ignored (`write_tool_schema_fixture`, the fixture writer).
- `cargo test -p lakehouse-api --lib chart_shapes_rewrite -- --test-threads=1` with `CHART_SHAPES_REWRITE_OUT` and `CHART_SHAPES_OUT` set (the tests write the statements the engine runs below): 7 pass.
- Integration files, each alone on the final tree (`cargo test -p lakehouse-api --test <name>`): `route_auth` 33, `sec11_guard` 4, `security_regressions` 10, `embed_tokens` 21, `query_author_exception` 6, `dashboard_export` 4, `ai_tool` 9, `upstream_errors` 6 passed, none failed. `route_auth` passed on its first run, alone. `sec11_guard` failed once on an earlier tree (a `#[cfg(test)]` module placed before non-test items in `routes/mod.rs`); the module is now declared last in `dashboard.rs` and the file passes.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (the six that were there before). `bun run test`: 1265 pass, 1 skip, 0 fail, 139 files (111 s). New test files: `src/lib/chart-shape.test.ts` (14), `src/features/dashboards/chart-shape-section.test.tsx` (12); added tests in `chart-option.test.ts` (5), `chart-click.test.ts` (1), `page-context-summary.test.ts` (1), `preview-key.test.ts` (1).
- Engine: `docker exec lakehouse-clickhouse-1 clickhouse-client -q "<statement>"` for every statement quoted below; read-only on `serving.*`; one scratch database `scratch_bi6_dev` (one table, 6 rows), dropped afterwards (`SELECT name FROM system.databases` shows it gone).

**Section 5, on the dev `ClickHouse` 26.8 (statements are the ones the tests print; numbers are the engine's)**

Table `serving.mart_demo_map_points` (237 rows, `visitors` 278,955, 26 provinces).

1. **`BI-6-AC4`, top 5 with Other.** Bar by `provinsi`, `sum(visitors)`, sort highest first, limit 5, `other: true`. The statement (`ranked.rs`):
   `SELECT __d AS provinsi, __m0 AS visitors, __o AS __other FROM (SELECT __d, __o, round(sum(visitors)) AS __m0, __m0 AS __t FROM (SELECT *, toUInt8(NOT (__k IN (SELECT __k FROM (SELECT *, ifNull(toString(provinsi), '') AS __k FROM serving.mart_demo_map_points) AS ob GROUP BY __k ORDER BY sum(visitors) DESC, __k LIMIT 5))) AS __o, if(__o = 1, 'Other', __k) AS __d FROM (SELECT *, ifNull(toString(provinsi), '') AS __k FROM serving.mart_demo_map_points) AS ob) GROUP BY __o, __d) ORDER BY __o, __t DESC, __d`
   Result: Dki Jakarta 82,088; Jawa Tengah 18,834; Jawa Barat 13,836; Sulawesi Selatan 12,799; Sumatera Barat 11,593; Other 139,805 (flag 1). 82,088 + 18,834 + 13,836 + 12,799 + 11,593 + 139,805 = 278,955 (and `SELECT sum(v)` over the same statement printed 278955). With `avg(visitors)` (kept groups by average): Riau 1,943; Sumatera Barat 1,932; Lampung 1,789; DI Yogyakarta 1,644; Sulawesi Selatan 1,600; Other 1,106. Direct: `SELECT round(avg(visitors)), count() FROM serving.mart_demo_map_points WHERE provinsi NOT IN (SELECT provinsi FROM serving.mart_demo_map_points GROUP BY provinsi ORDER BY avg(visitors) DESC, provinsi LIMIT 5)` returned `1106, 211`; the average of the 21 hidden averages would be 1,041. With a breakdown (`category`), Other per series was Office 32,915; Retail 23,878; Service point 46,715; Warehouse 36,297, equal to the direct `GROUP BY category` over the rows outside the five provinces.
2. **Chart filter and dashboard filter on one column (own: category Office, Retail; dashboard: Retail, Warehouse; the intersection is Retail, total 49,701 over 49 rows).** Each statement (printed by `the_statements_for_a_real_engine_run_are_built`) was run and compared with the direct query, the rows identical in all five forms: plain (`... WHERE category IN ('Office','Retail') AND category IN ('Retail','Warehouse') ...`, 19 provinces, sum 49,701), with a breakdown by `kab_kota` (26 rows, sum 49,701), with a month grain (15 buckets, sum 49,701; the predicates sit in `(SELECT * FROM serving.mart_demo_map_points WHERE ...) AS flt` under the bucketing, so they read the raw date), with a calculated field in the chart's filter (`is_big = If(visitors > 1000, 'big', 'small')`, own `is_big IN ('big')` with the dashboard's category filter: 20 provinces, sum 98,390, identical to the direct query), and over a SQL source (same 19 provinces, sum 49,701). A calculated field filter with Other: groups 62,900 + 14,652 + 12,799 + Other 135,369 = 225,720, the direct total of the big rows.
3. **Two aggregations of one column.** `measures: ["visitors", avg of visitors, max of visitors]` returned, e.g., Aceh 9,100 / 1,300 / 2,322 and Dki Jakarta 82,088 / 1,173 / 2,356 (columns `visitors`, `avg_visitors`, `max_visitors`). The first version of the statement (`round(sum(visitors)) AS visitors, round(avg(visitors)) AS avg_visitors`) failed on the engine with error 184 ("Aggregate function sum(visitors) is found inside another aggregate function"); fixed as deviation 9 and pinned by `a_measure_beside_another_aggregation_of_its_column_is_renamed_outside_the_aggregate`.
4. **The role rewrite on the new shapes** (real `sql_rewrite::enforce`, the function `rewrite_sql_for_roles` calls after it reads the policy; `routes/chart_shapes_rewrite.rs`). Two obligation sets on the demo table, both with a row filter `provinsi = 'Dki Jakarta'`: A masks `place` and `visitors`, B masks `place` and `category`. For the Other statement (sum, average, and per series), the sort beside a breakdown (by a measure and by the dimension), a table grouped by `category` and a monthly `visit_date` with two aggregations, a chart filter on a masked column, and the own-plus-dashboard filter, the tests assert that the table name appears exactly as many times in the rewritten statement as in the original (every read is the substituted projection), that each masked column's `replaceRegexpOne(toString(`col`), '(?s)^.*$', '***') AS `col`` appears once per read, and that `WHERE provinsi = 'Dki Jakarta'` appears once per read. All 14 combinations passed, and each rewritten statement was run on the engine. Under B (numbers readable): the sort beside a breakdown returned `Dki Jakarta, ***, 82088`; the Other statements `Dki Jakarta 82088 0`, `Dki Jakarta 1173 0` (average) and `Dki Jakarta *** 82088 0` (per series); the grouped table `*** 2026-10-01 10193 1133`, `*** 2026-06-01 9928 1241`, ... (only Dki Jakarta rows, category masked). Under A the engine refuses every sum with `Code: 43 ... Illegal type String of argument for aggregate function sum` because `visitors` is text after the mask; the plain statement `SELECT provinsi, round(sum(visitors)) AS visitors FROM (SELECT replaceRegexpOne(toString(visitors),'(?s)^.*$','***') AS visitors, provinsi FROM serving.mart_demo_map_points WHERE provinsi = 'Dki Jakarta') GROUP BY provinsi` fails with the same code, so a masked measure fails closed exactly as a plain chart's does. A filter on the masked column reads the masked value (the mask precedes `visitors >= 100` in the statement). I found no shape the rewrite cannot see into.
5. **A real value called "Other".** Scratch table `scratch_bi6_dev.t(p Nullable(String), v UInt32, c String)` with rows ('Other',100), ('A',50), ('B',40), ('C',30), ('D',20), (NULL,10); the Other statement with limit 2 returned `Other 100 0`, `A 50 0`, `Other 100 1` (the first the real value, flag 0; the last the computed group of B, C, D and the empty key: 100, flag 1). With limit 5 the empty key is its own group (`Other 10 1` computed from it alone), not lost. In the console the computed group is then drawn "Other (rest)" (the flag decides, not the label).
6. **Charts saved before this change.** Three stored definitions pinned in `charts_saved_before_bi_6_build_the_statement_they_always_did`, with the exact statement each builds (plain bar with a dashboard filter, a bar with a breakdown and a dashboard filter, a count with a value order and a dashboard filter) and the stored statement returned unchanged when no filter applies; the exact-string tests that existed before for grained, filtered, SQL-source and staged charts (`builder::tests`, `store::tests`, the BI-8 and BI-16 statements in `calc_fields`) still pass without an edit to any expectation. I did not build the base commit to diff statements byte for byte: the evidence is those pinned strings.

**Existing tests I had to change** (none weakened; no assertion lost)

- `routes/calc_fields.rs` helper `chart_sql`: `GrainedChart::from_def` takes the relation as a fourth argument now (the result names depend on it).
- `lakehouse-bi` tests that build a `ChartInput` by hand: `spec.def.measures = vec![..].into()` (three places), the `for_test` constructor gets `measures.into()` and `shape: ShapeFields::default()`. Assertions such as `assert_eq!(def.measures, vec![...])` compile unchanged because `MeasureList: PartialEq<Vec<String>>` (equal only when no measure has an aggregate of its own).
- `src/lib/preview-key.test.ts`: the `base` fixture gains `shape: ""` (the new required input); the "every input changes the key" test then covers it.
- `tests/fixtures/tool_schemas.json`: regenerated with `write_tool_schema_fixture`, then pinned by `tool_schemas_snapshot_is_byte_identical`.

**Deviations from the plan, and decisions I took where it was silent**

1. `Other` is offered for bar, hbar, stacked, pie, rose, funnel, treemap and a grouped table with one dimension only. Line, area, combo, scatter, bubble, waterfall, heatmap, radar, maps, sankey, sunburst, calendar refuse it with a plain sentence. The plan refuses it for a grain, a table calculation, a pivot and a raw table; it did not say which visual kinds take it.
2. A measure with its own aggregate is refused (plain sentence) on kpi, gauge, boxplot, point maps, pivot and a raw table; those kinds take the chart's aggregate.
3. `sort.by` is the dimension, a grouping of a table, or a measure's result name. Not the breakdown (refused).
4. A sort by a calculated aggregate is refused when the chart has a breakdown (a breakdown ranks by the dimension's total, which an arbitrary aggregate does not have). A chart with a table calculation takes no sort, `Other` or groupings.
5. Result names: an own aggregate `avg` of `stay` is `avg_stay`; a count is `count_<column>` and counts rows, as the chart's count does. Two entries with the same name are refused.
6. The `Other` flag is a result column `__other` (1 on the computed group), not a separate tile key. The console hides it from tables; it stays in "View data" and in the per-tile CSV, where it tells the computed group apart.
7. With a time grain, a sort by the dimension keeps the latest N buckets and shows them in the sort's direction; a sort by a measure is a value order (top N), as `order` already was.
8. Records: the route is tile-agnostic, so it gets an optional `chart` id and the server reads the tile's own filters; the console sends it only when the tile has filters of its own. Embeds and public links have no records list.
9. A bug found by the real-engine runs and fixed: a plain measure next to another aggregation of its column (`round(sum(v)) AS v, round(avg(v)) AS avg_v`) is engine error 184, because the second `v` is read as the first alias. Such a chart now aggregates under `__m<i>` names inside and renames outside (`select_lists`); a chart that cannot clash keeps its old statement.
10. A value list in the builder's filter editor reads every mart that has the column (no board yet); for a SQL source it lists nothing, and the text, number and date editors still work.

**Not verified** (and why)

- Anything in a browser: the dev API and console were not restarted and run the old build; the checklist below is for the reviewer or the owner.
- `AI-6-AC1` through the assistant (the model was not called): only the schema, its fixture, the registry test and the draft card's prefill are covered.
- `GET /api/dashboard/table-export` with a chart that has its own filters: the shared helper is tested through the records route (`records_pages`, 4 new tests), the export route itself needs a principal and an audit store and has no new test.
- An embed or public link in a browser (only `embed.rs`'s unit test, which shows the own filters applied beside the board's locked filters and `def.filters` in the payload); a masked role in a browser; `Other` on a large table (the statement reads the relation three times: kept groups, the flag, the rows; not timed).
- `cargo clippy` and `cargo test` over the whole workspace, `docker compose`, the gates (CI).
- The `__other` column is still in "View data" and in the per-tile CSV (it tells the computed group apart there); deliberate, not seen by the owner.

**Browser checklist** (the "Map demo" dashboard, id `b_980236ed`; the demo table has 237 rows, `visitors` sums to 278,955, 26 provinces, categories Office 63,696, Retail 49,701, Service point 89,447, Warehouse 76,111). Restart the API and the console on this tree first.

1. Open `/dashboards/b_980236ed`, press **New chart**, choose **Bar**. Data source `mart_demo_map_points`, Dimension `provinsi`, Measure `visitors`, Aggregation `sum`. Press **Add measure**, pick `visitors` and set that row's aggregation to `avg`. Expect the preview legend "visitors" and "avg of visitors"; Dki Jakarta: 82,088 and 1,173. Move the second measure up with its arrow: the legend order flips. Remove it: one series again.
2. Same chart, one measure `visitors`, **Sort** "visitors, highest first", **Limit** 5. Expect Dki Jakarta 82,088, Jawa Tengah 18,834, Jawa Barat 13,836, Sulawesi Selatan 12,799, Sumatera Barat 11,593. Turn on **Group the rest as "Other"**: a sixth bar, Other 139,805; the six add up to 278,955 (`BI-6-AC4`).
3. Change the measure's row to aggregation `avg` (or set Aggregation to `avg`) with **Sort** "avg of visitors, highest first": Riau 1,943, Sumatera Barat 1,932, Lampung 1,789, DI Yogyakarta 1,644, Sulawesi Selatan 1,600, Other 1,106 (the average of the other 211 rows, not of 21 averages, which would read 1,041).
4. Turn the "Other" switch off, set **Limit** 100, and under **Filters** use **Add a filter on a column**, pick `category`, choose Office and Retail, **Apply**. Expect the bars to add up to 113,397 (Office + Retail) and the chip "category is Office, Retail" under the list.
5. Create the chart. On the dashboard, the tile shows a filter icon; hover it: "category is Office, Retail". Add a dashboard filter **category is Retail, Warehouse**: the tile now shows Retail only, total 49,701 (49 rows); the dashboard filter chip can be removed, the tile's own filter cannot (`BI-6-AC5`).
6. Click the Dki Jakarta bar, **View records**: "Dki Jakarta" lists 17 records (Jakarta, Retail). Remove the dashboard filter: Jakarta then lists 32 records (Office and Retail). Click the **Other** bar: nothing opens.
7. **New chart**, **Data table**: under **Group by** pick `visit_date`, set it to **Month**, add `category`; measures `visitors` and (Add measure) `visitors` with `avg`; Filters: category Office, Retail; **Sort** "visitors, highest first"; Limit 10. Expect first rows 2026-03-01 Office 9,577 / 1,368; 2026-04-01 Retail 7,771 / 1,295; 2026-07-01 Office 7,681 / 1,536. Column headers read "visitors" and "avg of visitors". Download CSV: the headers and rows match.
8. Edit the chart from step 2 (**Edit chart**): the sort, limit, switch and filters come back as saved. Change **Sort** and watch the preview change; change the filter and watch it change (the preview follows every input).
9. As a user without `dashboard:write`: the builder's save fails with a plain message and nothing is stored (`BI-6-AC3`).
10. Ask the assistant: "Visitors by province and month, one category only, top 10". It should call `create_chart` with `filters`, `sort`, `limit` in one go (`AI-6-AC1`); not run in a browser or with the model by me.

## 8. Review (planner appends)

### BI-6 part 1 — smoke check and planner answers (reviewer, 2026-10-11)

By the owner's instruction for phase 2, there is no per-item browser QA and no reviewer re-run of the suites: the owner runs one QA at the end of the phase, and nothing is pushed before it. Part 1 is committed locally on that basis. What the reviewer did: rebuilt the dev API from this tree and probed it against the real engine.

- Top 5 provinces by `sum(visitors)` with `Other`: six rows adding up to 278,955 (`Other` 139,805, flagged).
- Two aggregations of one column with a chart filter (`provinsi` = Bali): `sum` and `avg` per category equal a direct query.
- The demo dashboard's 20-odd existing tiles still return no error.
- One headless screenshot of a saved tile with two measures, a chart filter, top 5 and `Other`: legend reads "sum of visitors" / "avg of visitors", the filter mark shows, `Other` is drawn last.

Not looked at by anyone: the builder dialog (measure rows, Filters section, sort, the `Other` switch, table groupings), the tile's list of its own filters when opened, embeds, a masked role.

Planner answers to the handoff: deviations 1, 2, 3, 5, 6, 7 and 8 are accepted as built. Deviation 4: `__other`, and any result column whose name starts with two underscores, is internal — hide it from "View data" and from the per-tile CSV as well (`SHOULD-FIX` R1, with part 2).
