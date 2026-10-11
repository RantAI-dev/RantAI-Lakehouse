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

## 8. Review (planner appends)
