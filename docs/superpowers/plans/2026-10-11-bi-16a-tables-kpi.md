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

## 7. Review (planner appends)
