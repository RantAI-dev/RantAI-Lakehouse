# BI-13: combine two sources on one chart — Implementation Plan

**Status:** planner defaults, not started. Written by the planner (Claude
Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/combined-charts.md`. Specs:
`docs/core/specs/bi-13.md`, `docs/core/specs/ai-6.md` (the `BI-13` row).

**Base:** `feat/bi-phase-2`, after `BI-11` is committed. Read first: the
phase 2 plans before this one and their handoffs; `lakehouse-bi`
`store.rs` (`ChartInput`, `spec_from_input`), `shape.rs`, `builder.rs`;
`routes/dashboard.rs` (`get_body`: one statement per tile, a tile's error
is its own) and `routes/embed.rs` (the phase 1 lesson: a failing optional
read must not fail the whole board); `chart-option.ts` (several measures
on one chart, added by `BI-6`), `chart-click.ts`, `drill.tsx`.

**Verification:** as for `BI-6`. Section 4 is not reduced.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Definition | Optional `layers` on a chart of kind `bar`, `line`, `area` or `combo`: 0 to 4 entries. A layer is a question of its own: `{ "mart" | "sqlSource", "dimension", "grain"?, "measures", "aggregate"?, "filters"?, "joins"?, "label"?, "draw": "bar" | "line", "axis": "left" | "right" }`. The base chart gains optional `draw` and `axis` for its own series. A chart with `layers` takes no `breakdown`, no `other`, no steps and no table calculation, and neither does a layer: refused with a plain sentence. |
| Matching | A layer's dimension must be of the same kind as the chart's (both dates, both text, both numbers). With dates, the layer uses the chart's effective grain (its own, or the dashboard's override), whatever it stored. Values are matched in the console on the result's dimension value. |
| Execution | Each layer is built and run as its own statement, through the same guard and role rewrite as a tile, with the dashboard's filters applied by the usual rule (a column the layer's source has). A layer's failure is recorded on that layer (fixed message with a reference, or the plain refusal) and never fails the tile or the board. Layers of one tile run concurrently with each other if tiles already do; otherwise in order. |
| Sort and limit | The base question's sort and limit decide which dimension values the chart shows. Each layer is asked for those values only when the base is cut by a limit on a text or number dimension (an `in` condition built from the base result, bound as literals, at most the chart kind's limit); for a date dimension the layer is asked for the same date range. |
| Payload | The tile's result carries `layers: [ { label, draw, axis, columns, rows, error?, source } ]` beside the base rows. Embeds and public links carry it too. |
| Drawing | One category or time axis; a right value axis only when a series uses it; each axis formats from its own series (`decimalsFor`). A value a layer lacks is a gap. A failed layer is listed in the legend with its reason. |
| Click | A click on a layer's mark offers that layer's own value drill: records from the layer's source with the layer's filters, and "Filter" on the dashboard by the dimension's column name. |
| Assistant | `layers`, `draw` and `axis` in the `create_chart` / `update_chart` schemas; `tool_schemas.json` regenerated. |
| Export | `chart_def_fields` lists `layers`, `draw`, `axis`. |

## 2. Tasks

- **T1 — `lakehouse-bi`:** `layers` on the definition, validation (kinds, counts, matching dimension kinds, the refusals), building each layer's statement with the chart's effective grain and the kept-values condition.
- **T2 — API:** running layers in the dashboard, preview, embed and public paths with per-layer errors; records for a layer (`chart` and `layer` index); YAML export; assistant schemas and fixture.
- **T3 — Console builder:** a "Questions" list on the supported kinds: add a question (source, dimension limited to matching kinds, measures, filters, draw, axis, label), reorder, remove; `previewKey` and `previewSpec`.
- **T4 — Console tiles:** merged drawing with two axes, gaps, the legend's failed-layer entry, per-layer click.
- **T5 — Docs:** `CHANGELOG.md` `[Unreleased]`.

## 3. Out of scope (do not build)

Matching on several dimensions; breakdowns in a combined chart; other chart kinds; computing across questions (a ratio of two questions is a join or a metric).

## 4. Things the developer must verify, not assume (not reduced)

Against the dev `ClickHouse` (26.8) and the real role rewrite, statements and results quoted:

- `BI-13-AC1` with `serving.mart_demo_map_points` by month and `serving.mart_demo_events` by month: each series equals a direct single query; months only one table has are gaps.
- A layer the caller's role cannot read is refused on that layer only; the base still returns rows. The kept-values condition does not let a layer see values through the base that its own row filter hides (the layer's statement is governed on its own).
- The kept-values literals are escaped: a base value with quotes or backslashes is matched, not interpreted.
- A board with a failing layer still renders in an embed (`tests/query_author_exception.rs` style).
- Charts saved before this change produce the same SQL and the same payload shape (no `layers` key added to them).

## 5. Handoff (developer appends)

### Partial notes (developer, uncommitted)

- 2026-10-11 T1 done (`lakehouse-bi`): new `layers.rs` (`LayerDef`, `check_definition`, `layer_input`/`layer_chart` = a layer read as the one-dimension bar chart it would be alone, `kept_values`/`kept_filter`, `bucket_range`/`trim_to_range`); `ShapeFields.{layers,draw,axis}` and the non-stored `layer_query` flag (always rebuild, no table calculation); `store::spec_from_input` now checks the definition, builds every layer over its own source with the chart's grain and compares dimension kinds (`checked_layers`); `fields.rs` refuses a table calculation in a combined chart. `cargo test -p lakehouse-bi`: 412 pass, 1 ignored; clippy and fmt clean. Next: T2.
- 2026-10-11 T2 done (API): new `routes/layers.rs` (`annotate_layers`, `layer_sources`, `LayerRunner`; every layer fails alone), wired into the dashboard read, the embed/public read (`board_columns`, `catalog_or_empty` split out of `render_board_payload`) and the builder preview; records take `layer`; `drillable` refuses a chart with layers; YAML export lists `layers`/`draw`/`axis`; `create_chart`/`update_chart` schemas and prompt; `filter_fields` offers the layers' columns. `routes/layers_rewrite.rs` = seven `#[ignore]`d tests that run the real pipeline through the real `enforce` against the real engine (section 4). Two embed tests in `tests/query_author_exception.rs`. Fixture regenerated (additions only).
- 2026-10-11 T3 and T4 done (console): `lib/chart-layers.ts`, `features/dashboards/{layers-section,combined-option}.tsx|ts`, builder wiring (`previewKey` has a `layers` input), `TileBody` draws the combined chart and lists failed questions, per-layer click (`ChartHit.seriesIndex`, `ClickContext.layer`, `RecordsRequest.layer`), `chart-draft-card`, `page-context-summary`, `drillBlockReason`. T5 CHANGELOG written.

### Handoff (developer, 2026-10-11, uncommitted, on top of `2ec061a`)

T1 to T5 are built in the working tree of `feat/bi-phase-2-build` (`/home/hv/lakehouse-phase2-build`). Nothing is committed, pushed or stashed. The dev servers (:3100, :18081) were not touched and run the old build, so **nothing here was seen in a browser and no route was called over HTTP**. No Postgres migration, no new `ClickHouse` table, no new route, no new assistant tool (the 79 stay 79).

**Files, by task**

- T1 (`lakehouse-bi`): new `layers.rs` (`LayerDef`, `check_definition`, `layer_input`/`layer_chart`, `kept_values`/`kept_filter`, `bucket_range`/`trim_to_range`, `base_filled_limit`, `same_dimension_kind` and its `KindMismatch`); `shape.rs` (`ShapeFields.layers`, `draw`, `axis`, and the never-stored `layer_query` flag that forces the builder to rebuild a layer's statement); `fields.rs` (a table calculation is refused in a chart that combines questions, with its own sentence); `store.rs` (`spec_from_input` is now the checked wrapper over `spec_from_input_base`: it checks the definition, then builds every layer over its own source with the chart's grain and compares dimension kinds, `checked_layers`; `spec_from_inline_sql` refuses layers); `lib.rs`.
- T2 (`lakehouse-api`): new `routes/layers.rs` (`annotate_layers`, `annotate_preview_layers`, `layer_sources`, the `LayerRunner` trait and `RoleRunner`) and `routes/layers_rewrite.rs` (tests); `routes/dashboard.rs` (layers in the read, `layer` on the records query, `layer_of`, `filter_fields` offers the layers' columns, YAML `layers`/`draw`/`axis`, preview); `routes/embed.rs` (layers in the embed and public read; `board_columns` and `catalog_or_empty` split out of `render_board_payload` for clippy's 100 lines, behaviour unchanged); `routes/drill.rs` (`drillable` refuses a chart with layers); `routes/ai/registry.rs` (`layers`, `draw`, `axis`; the filters and joins schemas became shared functions, output byte-identical for the old fields), `routes/ai/prompt.rs`; `routes/mod.rs`; `tests/fixtures/tool_schemas.json` (+562 lines, 0 deleted); `tests/query_author_exception.rs` (two embed tests).
- T3 and T4 (console): new `lib/chart-layers.ts`, `features/dashboards/layers-section.tsx`, `features/dashboards/combined-option.ts`; edited `chart-builder.tsx` (state, payload, `missing`, `previewKey`, the section), `tile-body.tsx` (`CombinedBody`), `echart.tsx` (passes `seriesIndex`), `chart-option.ts` (exports `PALETTE`), `tile-dialogs.tsx` (`Rows.layers`), `dashboard-page.tsx`, `drill.tsx`, `records.ts` (per-question click and records), `lib/chart-click.ts` (`ChartHit.seriesIndex`, `ClickContext.layer`), `lib/chart-shape.ts` (`ShapeDefFields`), `lib/preview-key.ts`, `lib/hierarchies.ts` (`drillBlockReason`), `lib/page-context-summary.ts` (the assistant's page context names each question and its rows), `features/copilot/chart-draft-card.tsx` (keeps `layers`, `draw`, `axis` of a draft). New tests: `chart-layers`, `combined-option`, `combined-tile`, `layers-section`; added cases in `preview-key`, `hierarchies`, `records`, `chart-draft-card`, `page-context-summary`.
- T5: `CHANGELOG.md` `[Unreleased]`.

**How it works (what the decision table left open)**

1. *A layer is read as the one-dimension bar chart it would be alone* (`layers::layer_chart`), through `stored_chart_sql` and `try_run_spec_sql`: the same builder, the dashboard's filters by the usual rule, the guard and the role rewrite of the layer's own tables. Nothing in `layers.rs` builds SQL text. `layer_query` on the synthetic chart makes the builder rebuild; an empty statement (a definition that cannot be built) is the layer's error, never "no rows".
2. *Which values a layer is asked for.* Text, number or ungrained date dimension: when the base filled its limit (rows >= limit, default 20, at most 100) the layer gets an `in` filter of the base's distinct values (bound by the ordinary predicate builder); otherwise it is asked for its own values, at most 100 rows, and the chart shows the union. Time grain that cuts to the latest buckets: when the base says `truncated` the layer is asked for every bucket (up to 1,000) and trimmed to the base's first and last bucket; a part of the date (hour of day) has its whole range.
3. *The grain.* A layer takes the chart's effective grain (own or the dashboard's switch) whatever its stored `grain`; a layer is never given the dashboard's switch a second time. A grain the layer's column cannot take (hour on a date) is that layer's error.
4. *Payload.* `layers: [{ label, draw, axis, source, dimension, columns, rows, error?, errorId?, filtersSkipped?, truncated? }]`; `dimension` (the column the rows are matched on) is an addition to the plan's list because it need not be named like the chart's. A tile with an error, and a chart without layers, get no `layers` key.
5. *Failure isolation.* Every failure is that layer's `error`: a refusal by the rewrite (its fixed sentence), a database error (the tile's fixed sentence and a reference, SEC-11), a source or table that is gone or whose list could not be read, a dimension that no longer lines up, an unbuildable definition. The list of SQL sources the layers need is read once per board by the layers' own code and its failure belongs to the layers that need it (the PR #104 lesson); in an embed the columns read is optional when only layers need it.
6. *Console drawing.* One category axis: the base's categories, then those only a question has (dates sorted); a missing value is `null`; a right axis only if a series uses it; each axis's `decimalsFor` from its own series; a failed question is listed under the chart with its reason and reference. A combo chart with questions keeps its first measure as bars on the left and the rest as lines on the right unless the chart's own `draw`/`axis` say otherwise.
7. *Click.* ECharts' `seriesIndex` tells the owner; a question's mark offers the dashboard filter on that question's dimension column and "View records" from that question's own source with `chart` and `layer`; the server uses that question's own filters and joins and refuses a source that is not the question's.

**Section 4, against the dev `ClickHouse` 26.8.9 and the real `sql_rewrite::enforce`** (`routes/layers_rewrite.rs`, seven `#[ignore]`d tests run by hand with `LAYER_ENGINE_URL/USER/PASSWORD` and `--ignored --test-threads=1 --nocapture`; they print the raw and the rewritten statement and the tile; `lakehouse-bi`'s `layers::tests` has two more against the real schema; read-only, nothing was created, so there is no scratch database to drop). The test role's policies were a row filter, a mask and an unreadable table, as noted.

1. **`BI-13-AC1`.** Base `serving.mart_demo_map_points` by month, layer `serving.mart_demo_events` by month (the dashboard's own statements):
   - base: `SELECT * FROM (SELECT visit_date, round(sum(visitors)) AS visitors FROM (SELECT date_trunc('month', visit_date) AS visit_date, visitors FROM serving.mart_demo_map_points) AS bkt GROUP BY visit_date ORDER BY visit_date DESC LIMIT 21) ORDER BY visit_date`
   - layer: `SELECT * FROM (SELECT event_time, round(sum(orders)) AS orders FROM (SELECT date_trunc('month', toTimeZone(event_time, 'Asia/Jakarta')) AS event_time, orders FROM serving.mart_demo_events) AS bkt GROUP BY event_time ORDER BY event_time DESC LIMIT 1001) ORDER BY event_time`
   - result: 15 base months (2025-09-01 18,148 ... 2026-11-01 11,299; sum 278,955) and two layer months (2026-08-01 11,781; 2026-09-01 11,259); each equals a direct single query on the engine (test asserts equality of the pairs). The other 13 months are absent from the layer (a gap, no zero). A `Date` and a `DateTime` column both print `YYYY-MM-DD` as month buckets (checked: `toTypeName` is `Date` for both).
   - base cut to 3 (`LIMIT 4`): `truncated: true`, buckets 2026-09-01 18,357, 2026-10-01 31,758, 2026-11-01 11,299; the layer (asked for all months) is trimmed to 2026-09-01 11,259 (August is outside the range).
2. **A layer the role cannot read is refused on that layer only.** With the events table unreadable for the role (`real_columns` unknown, so the rewrite refuses): the base returned 15 rows, layer 0 carried `error: "Query menyentuh tabel yang diatur kebijakan melalui konstruksi yang tidak didukung ..."` (the rewrite's fixed sentence, as a base tile would show) and no rows, and layer 1 (a count over the points table) answered 15 rows.
3. **The kept-values condition does not show what the layer's own rules hide.** Base: capacity by province, top 10 by hotels (Sulawesi Utara 195, Jawa Barat 188, Daerah Istimewa Yogyakarta 187, Nusa Tenggara Barat 182, Bengkulu 168, Maluku 150, Maluku Utara 149, Bali 146, Gorontalo 145, Kalimantan Timur 141); layer: visitors by province from the points table.
   - no policy: `... FROM serving.mart_demo_map_points WHERE provinsi IN ('Sulawesi Utara','Jawa Barat',...,'Kalimantan Timur') GROUP BY provinsi ORDER BY visitors DESC LIMIT 100` returned 8 provinces (Jawa Barat 13,836; DIY 8,221; Bali 7,997; Maluku Utara 7,931; Sulawesi Utara 6,757; Kalimantan Timur 5,719; Maluku 5,286; NTB 3,732; Bengkulu and Gorontalo have no visits).
   - row filter `provinsi = 'Bali'` on the layer's table: rewritten to `FROM (SELECT ... FROM serving.mart_demo_map_points WHERE provinsi = 'Bali') mart_demo_map_points WHERE provinsi IN (...)`; the layer returned Bali only. Row filter `provinsi <> 'Bali'`: the other 7 provinces, Bali absent. An earlier run of the same test with the row filter `provinsi = 'Dki Jakarta'` (not among the base's ten) returned no layer rows.
   - mask on `provinsi`: `replaceRegexpOne(toString(`provinsi`), '(?s)^.*$', '***') AS `provinsi`` inside the projection, the `IN` evaluated outside it on the masked value: no row came back, so the base's real names probed nothing and revealed nothing.
   The row filter and the mask are applied beneath the condition; the condition only narrows.
4. **Escaping.** Five base values `O'Brien`, `back\slash`, `x') OR 1=1 --`, `日本`, `plain` (SQL sources of literal rows, base limit 5 so the base is full): the layer statement was `... WHERE d IN ('O''Brien','back\\slash','plain','x'') OR 1=1 --','日本') ...`, rewritten to `IN ('O\'Brien', 'back\\slash', 'plain', 'x\') OR 1=1 --', '日本')`; the layer returned exactly those five rows with the values intact, and `extra` (a sixth value of the layer's source) was absent. A number dimension: base `n` 0..3 (limit 4) gave `WHERE n IN ('0','1','2','3')` and the layer returned n = 0,1,2,3. A unit test pins the escaping of the filter alone (`d IN ('a','O''Brien\\ x'') OR 1=1 --')`).
5. **A board with a failing layer still renders in an embed.** Two tests in `tests/query_author_exception.rs` (mock `ClickHouse`, signed embed token): a layer whose table fails with a planted engine error answered with the tile's fixed sentence and `errorId`, no engine text in the body, the base rows present and status 200; a layer on a SQL source whose list cannot be read answered "this question's SQL source could not be read." with the base rows present. Also `a_chart_without_layers_asks_nothing_and_gets_no_layers_key` and `a_failing_or_refused_statement_is_that_layers_sentence_and_its_neighbours_answer` (stub runner).
6. **Charts saved before this change.** The 412 `lakehouse-bi` tests include the pinned statements of `BI-6` to `BI-11` unchanged; `layers::tests::a_combined_chart_saves_with_the_statement_it_would_have_alone` (engine) saves the same chart with and without layers and compares `spec.sql`: identical, and the chart without layers serialises without `layers`, `draw` or `axis`; `tool_schemas.json` gained 562 lines and lost none; the YAML export test pins that the three keys appear only when present.
7. **Save path against the real schema** (`layers::tests`, engine): the AC1 chart saves; refusals, as written: a text layer under a month grain "question 1: a grain needs a date or timestamp dimension."; a timestamp against a date with no grain "question 1: its date column and the chart's are not the same type (a date and a timestamp); choose a time grain so they line up."; a missing measure "question 1: invalid or missing measure column."; a missing table "question 1: Gold mart 'mart_gone' not found in serving."; a missing source "question 1: SQL source 's_gone' not found."; an hour grain over a layer of dates "question 1: hour needs a timestamp column; this column has no time."; a breakdown "a chart that combines questions has no breakdown, 'Other' group or steps."; a pie "questions can be added to a bar, line, area or combo chart only."

**Commands run, with counts** (rust from `/home/hv/lakehouse-phase2-build/rust`, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=4`, one cargo at a time, `-p` scoped, `lib.rs` of both crates touched before clippy; `rust/target` never appeared)

- `cargo fmt --check` (whole tree): clean; I ran `cargo fmt -p lakehouse-bi -p lakehouse-api`, and the tree was fmt-clean before, so every hunk it touched is mine.
- `cargo clippy -p lakehouse-bi -p lakehouse-api --all-targets -- -D warnings`: clean (it found: a function over 100 lines in `render_board_payload`, split; a match with two identical arms; names too alike in tests; by-value test arguments).
- `cargo test -p lakehouse-bi`: 412 lib pass, 3 ignored (the two engine tests of `layers` and the earlier one); `specs_match_typescript` 1 pass. `cargo test -p lakehouse-api --lib`: 1636 pass, 8 ignored (the seven engine tests of `layers_rewrite` and the fixture writer). Each alone: `route_auth` 36 pass, `sec11_guard` 4, `query_author_exception` 8 (6 + 2 new), `embed_tokens` 21, `dashboard_export` 6, `ai_tool` 9. Run by hand against the dev engine with `--ignored --test-threads=1`: `lakehouse-bi` `layers::tests` 2 pass, `lakehouse-api` `layers_rewrite` 7 pass. Not run (not on the brief's list): `security_regressions`, `upstream_errors`, `reporting_settings_route`, `lakehouse-alerts`, `lakehouse-store`.
- `bun run typecheck` clean; `bun run lint` 0 errors, the same 6 warnings; `bun run test`: 1395 pass, 1 skip, 0 fail, 1396 tests in 157 files (109 s). New test files: `chart-layers` 7, `combined-option` 8, `combined-tile` 3, `layers-section` 9; added cases in `preview-key` (1), `hierarchies` (1), `records` (2), `chart-draft-card` (1), `page-context-summary` (1).
- Engine: `docker exec lakehouse-clickhouse-1 clickhouse-client` (26.8.9) for the month buckets and the totals of the checklist, and the tests above through `localhost:8123` with the credentials of the worktree's `.env`; read-only, no table or database created.

**Existing tests changed** (none weakened, no assertion lost): `preview-key.test.ts` base object gains `layers: ""` (a new required input); nothing else. `routes/mod.rs` first declared the new test module in the middle of the file and `sec11_guard` refused it (a non-test item after a `#[cfg(test)]` module); it is now declared at the end of `layers.rs`, as `dashboard.rs` does for its own. `sec11_guard` also rejected an error sentence built with `{why}`; the reason is now a fixed sentence from an enum (`KindMismatch`), and no allowlist entry was added.

**Deviations and decisions where the plan was silent** (planner to accept or reverse)

1. The payload entry carries `dimension` (above).
2. *Bins* on the chart are refused with layers (the plan listed breakdown, "Other", steps and table calculation); a bin label can never equal another source's value.
3. An *unsaved custom SQL* base refuses layers ("save the SQL as a source before combining it with other questions."), because the questions are checked against stored sources and tables.
4. `GET /api/dashboard` `filterFields` also offers the columns of the layers' tables (mart layers without joins), counted once per tile, so a dashboard filter can be made on a column only a question has.
5. The layer's stored `grain` is validated as a name and kept, but the read ignores it (as the table says).
6. A layer asked for its own values (base not full) returns at most 100 rows (1,000 buckets with a grain), by value, and says nothing when it was cut at 100; with a grain the cut is flagged `truncated` on the layer.
7. The first question's `limit` is also the limit of the kept values: at most 100.
8. A layer's measures are not capped at a number beyond the builder's six; layers have no sort or limit of their own.

**Not verified**

- Anything in a browser: the "More questions" section, the picker of a question's columns (the fields route for a joined layer), the drawing (two axes, gaps, the failed-question strip, the legend), the per-question click menu and its records list, the builder's preview with layers, the assistant card for a `create_chart` with layers, an embed or a public link in a browser.
- No route over HTTP: the preview, the dashboard, the records with `layer`, the YAML export and the embed were exercised through their functions and mock servers, not over a running API (the dev API was not rebuilt).
- A layer over a SQL source *and* a base over a mart through the dashboard (the engine tests used SQL sources of literal rows on both sides; a mart base with a SQL-source layer was exercised only with the stub and the embed mock); a layer with joins; a layer over a calculated field or a value group as its dimension (the kind comparison is skipped when the kind is not known); a kept-values condition on an ungrained date dimension (the condition is the ordinary `in`, not run against a `Date` column here).
- The assistant through the model (`AI-6-AC1`: "visitors by month next to rooms"); the schema and prompt were not run against a model.
- `security_regressions`, `upstream_errors`, `lakehouse-alerts`, workspace-wide clippy and test, `docker compose`, the gates (CI).
- Disk: free space fell to 12G during the integration-test builds; I removed `incremental` and two test executables over 20M after the first full run (21G free again).

**Browser checklist** ("Map demo", `/dashboards/b_980236ed`; rebuild and restart the API and console on this tree first; numbers from the dev engine: `serving.mart_demo_map_points` 237 rows, 278,955 visitors, 15 months from 2025-09 to 2026-11; `serving.mart_demo_events` 1,440 hourly rows from 2026-08-01, 23,040 orders in August (11,781) and September (11,259) 2026, channels App 7,680, Store 7,659, Web 7,701)

1. As an editor open the dashboard, **New chart**, **Bar**, source `mart_demo_map_points`, dimension `visit_date`, **Group by** Month, measure `visitors` (sum), Limit 20, Sort natural. Preview: 15 bars, `Sep 2025` 18,148 ... `Nov 2026` 11,299.
2. Scroll to **More questions**, **Add a question**: source `mart_demo_events`; **Lined up on** offers only `event_time` (with a month grain a timestamp lines up with a date); measure `orders` (sum); legend name `Orders`; **Line**, **Right axis**. Preview: the bars plus a line with two points, `Aug 2026` 11,781 and `Sep 2026` 11,259, the line absent from the other 13 months (gaps, not zeros), a right value axis, a legend with `visitors` and `Orders`; the two axes format differently if you choose an average with decimals.
3. Set **Group by** to none: **Lined up on** now lists no column of the events table (a timestamp does not line up with a date without a grain), says so, and the question's dimension empties; the preview asks for it ("Question 1 dimension"). Set Month again and pick `event_time`.
4. Set **Limit** to 3 (Sort natural): bars `Sep 2026` 18,357, `Oct 2026` 31,758, `Nov 2026` 11,299 with the cut marker; the Orders line shows `Sep 2026` 11,259 only (August is before the range).
5. Add a second question on `mart_demo_events` with measure `orders` (count) and a filter `channel` is `App`, a Bars/Left axis: it draws beside the others; reorder it with the arrows; remove it. Add a fifth question after four: the button disappears at four.
6. **Create**. On the dashboard click the `Sep 2026` bar: the menu offers **Filter dashboard by this** (a range on `visit_date`, September 2026) and **View records** (rows of the points table in September, 16 rows). Click the `Sep 2026` point of the Orders line: **Filter dashboard by this** on `event_time` and **View records** listing rows of `mart_demo_events` in September 2026 (703 rows from 2026-09-01 in the report zone), never the points table; there is no "Drill down" line on either.
7. Add a dashboard filter `channel` = `App`: the Orders line becomes `Aug 2026` 3,908 and `Sep 2026` 3,772; the bars keep their numbers and the tile's marker says the base cannot take the filter. A filter on `visit_date` (last 6 months) narrows both series.
8. Edit the chart, choose text: dimension `provinsi` on `mart_demo_province_capacity`, measure `hotels`, Limit 10, Sort highest first, and add a question on `mart_demo_map_points` lined up on `provinsi`, measure `visitors`, a line on the right axis: ten bars (Sulawesi Utara 195, Jawa Barat 188, Daerah Istimewa Yogyakarta 187, Nusa Tenggara Barat 182, Bengkulu 168, Maluku 150, Maluku Utara 149, Bali 146, Gorontalo 145, Kalimantan Timur 141) and a line with eight points (Jawa Barat 13,836; Daerah Istimewa Yogyakarta 8,221; Bali 7,997; Maluku Utara 7,931; Sulawesi Utara 6,757; Kalimantan Timur 5,719; Maluku 5,286; Nusa Tenggara Barat 3,732), none for Bengkulu and Gorontalo. Set the limit to 50: the base has 28 provinces, does not fill its limit, so the line also shows provinces outside the first ten (every province with visits).
9. Try a breakdown on the chart with a question present: the section says "Remove a breakdown to combine questions." and the preview shows the server's sentence. Change the kind to Pie: the section disappears and the questions are not saved.
10. As a user with `dashboard:read` only: the tile draws all questions, the click menus work, there is no builder; as a user whose role cannot read `mart_demo_events` (a policy on it): the bars draw and the strip under the chart lists `Orders:` with the policy sentence; a role with a row filter on the points table shows only its rows in the bars and in a text-dimension line.
11. **Share** the dashboard (embed or public link): the same chart with the questions; take the events table out of the embed role's reach (or stop its source): the bars still draw and the strip lists the failed question with a reference.
12. **Export** the dashboard YAML: the chart lists `layers` with each question's `mart`, `dimension`, `measures`, `draw`, `axis`, `label`; a chart without questions lists no `layers`, `draw` or `axis`.
13. Ask the assistant: "visitors by month next to the orders" on this dashboard (`AI-6-AC1`): expect `create_chart` with the points chart by month and one entry in `layers` for `mart_demo_events` (not run by me).

## 6. Review (planner appends)

### BI-13 — planner answers (reviewer, 2026-10-11)

No per-item browser QA and no reviewer re-run of the suites (owner's instruction for phase 2). Planner answers to the handoff: deviations 1 to 4 and 6 to 8 are accepted as built.

- `SHOULD-FIX` R1 (deviation 5) — a layer asked for its own values is cut at 100 rows without a mark. A cut layer must carry `truncated` on its payload entry and the tile's legend must say so, like any cut tile. To be fixed at the start of the next item.

The reviewer's smoke check on a running API is recorded in the next entry.

### BI-13 — smoke check (reviewer, 2026-10-11)

On the dev API rebuilt from `52ccac2`, over HTTP against the real engine: a bar chart of visitors by month with one layer (orders by month from a second table, line, right axis) saved; the dashboard returns 15 base rows and the layer's two months, 11,781 and 11,259, equal to a direct query; no tile of the demo dashboard returns an error. One headless screenshot: bars on the left axis, the layer's line on the right axis with its own scale, the other months left as gaps, both in the legend.

Not looked at by anyone: the "Questions" section of the builder, a failed question's line under the chart, a click on a layer's mark.
