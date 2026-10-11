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

## 6. Review (planner appends)
