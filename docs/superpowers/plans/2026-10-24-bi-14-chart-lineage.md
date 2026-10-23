# BI-14: lineage from a chart to its sources (with AI-13)

Planner and reviewer: Claude Opus. Developer: a non-Opus coding agent.
Branch: `feat/bi-phase-6` (to be cut; shared with `BI-16` C and `BI-17` B),
base `feat/bi-phase-4` until phase 4 merges, then `main`. Feature page:
`docs/core/features/chart-lineage.md` (decision 1 waits for the owner, Q6;
the rest planner defaults confirmed at the phase 6 QA). Spec:
`docs/core/specs/bi-14.md`; assistant `docs/core/specs/ai-13.md`.
**Blocked by `DATA-13` (column part): cleared**, `DATA-13` merged in #106
(In Acceptance); its code is on this base (`routes/lineage/columns.rs`).

Anchors were read on 2026-10-23 on `feat/bi-phase-4` at `150a081a`. Find
each by the symbol named. New lineage code goes in `routes/lineage/`, not
in `lineage.rs` (that file is upstream-shaped and already long).

## 1. Decisions already made (do not re-ask, do not change)

The feature page's table, plus these build decisions.

- **D1 Chart route.** `GET /api/dashboard/charts/{id}/lineage`
  (`lineage:read` per Q6 as signed; both ways in `route_auth`). The chart
  is loaded through the person's `Viewer` (`viewer_of`) so a chart in
  someone else's personal space reads as not found. Answer: `{ chart,
  steps: [{ id, kind, label, refreshedAt?, refreshSource?, notRecorded? }],
  edges: [...], fields: [{ field, columns: [{ table, column, via }] }],
  coverage: [...], supported: true }`. Module
  `routes/lineage/charts.rs`.
- **D2 Start nodes.** From the stored chart (`StoredChartSpec`):
  `def.mart` → `serving.<mart>`; `def.sql_source` → the column-lineage
  key `sql_source_key(id)` and, through `column_lineage` rows, the tables
  it reads; `shape.joins_list()` tables; `shape.layers_list()` (each
  layer's mart or SQL source); measures that reference a metric →
  `metric:<id>` → the metric's mart. From these, the existing table graph
  (`lineage::build`'s `Graph`; its `reachable` walks both ways, so the chart route uses an upstream-only walk: a `forward: false` pass of the same function, parameterised, not a copy) adds
  Silver, Bronze, connector, upload and pipeline nodes. No second graph
  builder: `build` is refactored to expose the assembled `Graph` to the
  new module (`pub(crate)` constructor), not copied (rule 4).
- **D3 Refresh times.** Gold/Silver: `builds.rs` `BUILDS_SQL`'s `at`
  (already read for the graph); Bronze: the Iceberg `last_updated_ms` the
  catalog reads (`routes/catalog.rs` freshness, the same helper, not a new
  Iceberg call); connector: last successful run from
  `lakehouse_store::connector_run`; upload: its claim's time. Each step
  carries `refreshSource` naming the record. No record → `notRecorded:
  true` and a `coverage` line.
- **D4 Columns.** For each of the chart's fields (dimension, measures,
  series, calculated fields), the columns from `columns::mappings_touching`
  on the start tables, walked upstream as far as mappings exist; a
  calculated field's referenced columns from its compiled formula
  (`lakehouse_bi::formula`). Fields with no mapping say so.
- **D5 Used by.** `GET /api/governance/lineage/used-by?table=` (`lineage:read`):
  every stored chart whose start nodes (D2) include the table, directly or
  through a SQL source or metric, filtered by `Viewer`, up to 200, with
  `hiddenCount` for those filtered out by visibility, and their boards.
  Implemented in `routes/lineage/used_by.rs` from
  `store::list_stored_charts` and `store::list_boards` for the viewer, plus
  one unfiltered count (no names) for `hiddenCount`. The table graph gains
  chart, SQL source and metric node kinds so `?focus=` on the lineage page
  shows them (downstream edges table → chart).
- **D6 Assistant (AI-13).** Tool `get_chart_lineage { chart }` (ReadOnly)
  calling the D1 code with the person's principal and the same permission
  check (the tool refuses with the route's sentence without
  `lineage:read`); `get_lineage` adds `usedBy` (D5) for a table focus.
  Prompt line; `capabilities.ts`; fixture regenerated with the ignored
  writer test; count pin +1; the `AI-13-AC1` case in
  `ops/ai_eval/dashboard_cases.json`.
- **D7 Console.** Tile menu item "Where does this come from"
  (`dashboard-page.tsx` tile menu), opening a side panel
  (`src/features/dashboards/chart-lineage-panel.tsx`) through a new
  `dashboardService.chartLineage(id, signal)` (contracts → client →
  `services/index.ts`); "Open in lineage" links to
  `/lineage?focus=chart:<id>`; Data Explorer's asset page and the lineage
  page gain a "Used by" list. Hidden in embeds and public links.

## 2. Anchors (verified at `150a081a`, find by symbol)

- Lineage: `routes/lineage.rs` module doc (edge/record table), `Graph`
  (`node`, `edge`, `coverage`), `table_id`, `zone_node`, `reachable`,
  `matches_focus`, `build` (`MAX_NODES` 150), `add_shared`,
  `add_tenant_owned`; `routes/lineage/builds.rs` `BUILDS_SQL` (the `at`
  column), `drop_repeats`; `routes/lineage/columns.rs` `sql_source_key`,
  `record_sql_source`, `mappings_touching`, `read`, `graph_key`,
  `zone_key`; `lakehouse_store::column_lineage` (`MappingRow`,
  `Direction`, `Scope`).
- Routes and policy: `routes/governance.rs` `lineage`, `column_lineage`;
  `routes/mod.rs` (`/api/governance/lineage` and `/columns` registered
  before `{kind}`: the new `used-by` path goes with them); `policy.rs`
  rows for both (`lineage:read`); seeds `0002_seed_identity.sql`
  (`Analyst` has `lineage:read`, `Dashboard Viewer` has only
  `dashboard:read`).
- Charts: `lakehouse-bi/src/store.rs` `list_stored_charts`,
  `list_boards`; `routes/dashboard_organise.rs` `viewer_of`;
  `routes/dashboard.rs` `effective_chart`; chart shape `joins_list`,
  `layers_list`; `routes/layers.rs` `layer_sources`.
- Freshness sources: `routes/catalog.rs` (Bronze `freshnessLagSeconds`
  from Iceberg `last_updated_ms`); `lakehouse-store/src/connector_run.rs`
  `list_runs`.
- Assistant: `routes/ai/tools/data.rs` `get_lineage`;
  `routes/ai/registry.rs` `get_lineage_schema`, `TOOLS`;
  `capabilities.ts`; `page-context.ts`.
- Console: `src/features/governance/lineage-page.tsx` (`?focus=`),
  `lineage-graph.tsx`, `lineage-columns.tsx`; Data Explorer asset page
  (`src/features/catalog/asset-detail-tabs.tsx`, lineage tab);
  `dashboard-page.tsx` tile menu.

## 3. Tasks (one commit each, in order; Rust first)

**T1 Graph reuse and new node kinds (D2, D5 graph part).** Expose the
assembled graph; add chart, SQL source and metric nodes with downstream
edges from their tables, each edge's `evidence` naming the stored chart
or source. *Accept:* existing lineage tests unchanged and green; a test
graph with a chart over a mart, one over a SQL source and one with a
metric gives the expected nodes and evidence; `MAX_NODES` still bounds.

**T2 Chart route (D1, D3, D4).** *Accept:* route tests: path from a chart
to a connector (fixture records), refresh times with their sources, a
step with no record says so; a chart in another person's personal space
→ 404; without `lineage:read` → 403 with the sentence; columns per field
from fixture mappings, a calculated field's columns; SEC-11 guard green;
`route_auth` both ways.

**T3 Used by (D5).** *Accept:* lists charts directly on the table, via a
SQL source, via a metric; another person's personal chart not listed and
counted in `hiddenCount`; 200 cap; `route_auth` both ways.

**T4 Assistant (D6).** *Accept:* tool tests (path for a chart, refusal
without `lineage:read`, `usedBy` on `get_lineage`); fixture; count pin;
`capabilities.ts` test; AI-13-AC1 case.

**T5 Console (D7).** *Accept:* panel component tests (steps, "not
recorded", refusal sentence for a viewer without the permission, open in
lineage link); lineage page renders chart nodes; "Used by" on the asset
page; no menu item in the embed view; typecheck, lint, tests of the
touched files; webpack build.

## 4. Out of scope

Filter and parameter lineage, column-level impact lists, inferring edges
the platform did not record, lineage in embeds or public links.

## 5. Must verify, not assume

- Whether the assistant's existing `get_lineage` checks `lineage:read`
  (it calls `lineage::build` with the principal, but the permission is a
  route policy); if it does not, that is a pre-existing gap: report it
  and add the check in T4 with a test (rule 13 citation: this plan).
- How a measure names a metric in the stored chart (the field catalog
  path `annotate_metric_labels` reads) so D2 finds metrics.
- That the Bronze freshness helper in `catalog.rs` can be called for one
  table without listing the catalog.
- That `system.query_log` retention on the deployment matches the 180-day
  bound of `BUILDS_SQL` (the feature page's limit says half a year).

## 6. Checks

As in the phase 4 plans (same cargo environment:
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2
CARGO_INCREMENTAL=0`, one cargo command at a time, `--test-threads=4`,
`route_auth` at 2; check `free -g` / `df -h /` first; never `cargo build -p
lakehouse-api`; no `rust/target`; no prettier; touch the crate sources
before clippy when several worktrees share the target dir). Per task the
crates touched (`lakehouse-bi` if the store gains a read, `lakehouse-api`),
the `lakehouse-api` lib filters `lineage`, `governance`, `ai`, the
integration files `route_auth`, `route_policy`, `sec11_guard`, `ai_tool`
and the lineage integration tests. Once, before the handoff: `cargo fmt
--check` and whole-workspace `cargo clippy --workspace --all-targets
--all-features -- -D warnings` with the environment above (no full
workspace test run on this machine; CI runs it on the PR). TS:
typecheck, lint, tests of the files touched; the webpack build after T5.
No migration is expected; if one turns out to be needed: next free number
at build time, check every branch (`0098`+ are being taken by `BI-19`).

## 7. Handoff (developer appends)

## 8. Review (planner appends)
