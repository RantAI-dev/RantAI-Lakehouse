# `BI-71`: lineage that reads at a glance (chart panel and Catalog map)

Planner and reviewer: Claude Opus. Developer: an Opus developer agent
(console design work, `AGENTS.md`). Base: `feat/bi-59-dashboards-page` at
`9db4fcec0` (it has `listChartOutlines`, decision 4). Branch:
`feat/bi-71-lineage-readable`. **Merge after #530.**

## Why

Owner acceptance of `BI-14`, 2026-11-20 (QA stack, chart "qa-auto metric
chart" on "QA Auto", table `serving.mart_wisman`). The data is right; the
look is not:

1. **"Where does this come from"** (`src/features/dashboards/chart-lineage-panel.tsx`)
   is five stacked bordered cards with "Not recorded" on every card, "No
   column mapping recorded" under every field, and a bulleted paragraph of
   technical coverage notes ("kurang menarik, kek cuma card card doang").
2. **The Catalog Lineage tab** (`src/features/catalog/asset-lineage-map.tsx`,
   `LineageMindmap` at :281) draws every chart that reads the table as its
   own node: `mart_wisman` has 111, a column the height of many screens, in
   a fixed-size `overflow-x-auto` box (:289) with no zoom ("flownya panjang
   dan banyak banget… gabisa di zoom out lagi").

The owner chose **mockup A, "Route"** for the panel (2026-11-21; options A–D
on the local branch `design/chart-builder-mockups`, commit `6c9456aee`, files
`src/features/mockups/chart-lineage/mockup-a.tsx`, `shared.tsx`,
`fixture.ts`, `chart-lineage.css`). D (A + C's flowing edges) was built and
rejected ("jelek ternyata").

## Decisions (planner, 2026-11-21)

1. **The panel is mockup A, built on the real data.** Read `mockup-a.tsx`
   and port its look and motion, not its fixture. One vertical route from
   the source at the top down to the chart at the bottom (the reverse of
   today's order): each step is a small kind icon in a circle on one
   connecting line, the name, and the kind; the line draws downward on open
   (A's stagger, transforms only, ≤250 ms per step, none under
   `prefers-reduced-motion`). A step shows its refresh time
   (`refreshWords`) **only** when it has one; the fields sit as chips on
   the chart step, a recorded mapping written `field ← column` with the
   full `table.column` in its tooltip. Whatever is missing is said **once**,
   in one quiet line under the route ("No refresh times or column mappings
   recorded yet", or "Only recorded times and columns are shown" when some
   are), ending in the existing `HelpHint`
   (`@/components/patterns/help-hint`) that holds the server's `coverage`
   sentences. "Open in lineage" stays the one action, at the bottom.
   Refused, loading, error and unsupported states keep their behaviour; the
   loading placeholder takes the route's shape (DESIGN: one page, one
   loading look). The sheet keeps `sm:max-w-md` and works at 390 px.
2. **The Catalog map groups charts by dashboard.** The chart nodes that read
   the asset become one node per dashboard: "<dashboard title> · N charts",
   with the dashboard's icon, ordered by N then title. Clicking (or Enter
   on) a group opens it in place to list its charts as today's nodes, and a
   second click closes it; one group open at a time. Charts the viewer
   cannot place on a readable dashboard go in one "Other charts · N" group.
   No other node kind is grouped.
3. **The map pans and zooms, and opens fitted.** Drag (mouse, touch) pans;
   the wheel with Ctrl/⌘, a trackpad pinch and a phone pinch zoom around
   the pointer; buttons `+`, `−` and **Fit** (ghost icon buttons, 32 px,
   tooltips with the shortcut) sit in the map's corner. The map opens fitted
   to its box; the minimum zoom is whatever fits the whole graph (never less
   than 0.2), the maximum 2. Fit and zoom changes ease over 200 ms (none
   under reduced motion). The box keeps today's height rules; no page
   scroll is trapped (a plain wheel scrolls the page, Ctrl/⌘+wheel zooms).
   Keyboard: `+`/`-`/`0` (fit) while the map has focus.
4. **Where the dashboards come from.** Map chart node ids (`chart:<id>`) to
   their board with `dashboardService.listChartOutlines()` (one read of the
   specs list, BI-59) and to a title with `dashboardService.listBoards()`.
   Both are the viewer's own readable lists, so a private dashboard's title
   never appears for someone who cannot open it. No API change. A failed
   read of either falls back to today's ungrouped nodes, quietly.
5. Out of scope: the full lineage page (`/lineage`), the API, column-level
   lineage, any wording beyond decision 1's two lines.

## Tasks

- **T1** Panel (decision 1) with tests: steps render source-first; a step
  with no refresh time shows no time and no "Not recorded"; the one missing
  line appears once with the hint holding `coverage`; a mapped field chip
  reads `field ← column`; refused/unsupported unchanged.
- **T2** Grouping (decisions 2, 4) in `mindmapModel`/`LineageMindmap`, with
  tests: 111 charts on 3 dashboards become 3 group nodes plus "Other
  charts"; opening a group lists its charts; a failed outline read keeps
  the ungrouped map.
- **T3** Pan and zoom (decision 3), with tests for Fit on open, the zoom
  bounds and the buttons; check by hand at 1190×800 and 390 px.
- **T4** `CHANGELOG.md`; the `BI-71` backlog row stays as the planner wrote it.

## Checks (this VM)

Never the full `bun run test` or `bun run lint`; wrap every bun command in
`flock /tmp/claude-1000/lh-bun.lock systemd-run --user --scope -q -p
MemoryMax=3G -p MemorySwapMax=0 nice -n 15 …`. Per commit:
`bun x tsc --noEmit -p .`, `bun x eslint <changed files>`, `bun test
<the dashboards lineage panel and catalog lineage test files>`. No prettier,
no cargo, no dev server.
