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

## 6. Review (planner appends)
