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

## Handoff

Developer: Opus developer agent (console design work), 2026-11-20. Branch
`feat/bi-71-lineage-readable`, not pushed.

### Commits (plan order, one task each)

- `512b69182` T1 `BI-71 feat(dashboards): chart lineage panel reads as one route`
- `98d84e091` T2 `BI-71 feat(catalog): group the lineage map's charts by dashboard`
- `edd5d0694` T3 `BI-71 feat(catalog): the lineage map pans, zooms and opens fitted`
- `365ee4f57` T4 `BI-71 docs(changelog): lineage that reads at a glance`
- this commit: the handoff.

### What was built

- **T1** `src/features/dashboards/chart-lineage-panel.tsx`: mockup A's
  route on the real `ChartLineage` (steps reversed to source first; kind
  icon per step, the chart step "This chart" with its fields as chips;
  refresh time only via `hasRefreshTime`, relative in the line and
  `refreshWords` in the tooltip and for screen readers; the one missing line
  from `missingWords` with `HelpHint` holding `coverage.join(" ")`; "Open in
  lineage" pinned under the scroll area). Motion: three `@theme` animations
  in `src/app/globals.css` (`route-draw`, `route-pop`, `route-rise`, 240 ms,
  `ease-ui`, transforms/opacity only; mockup's `cl-draw` was 320 ms, cut to
  240 per decision 1), staggered 70 ms; the existing global
  `prefers-reduced-motion` rule stops them. `src/lib/lineage-words.ts`
  gains `hasRefreshTime` (now also used by `refreshWords`) and
  `missingWords`. A field with several recorded columns shows the first in
  the chip and all of them in the tooltip.
- **T2** `src/features/catalog/asset-lineage-map.tsx`: `chartPlacement`,
  `groupCharts` (nodes with id `chart:*` only; group level = the deepest of
  its charts; an open group's charts one column to its right, joined to
  it), `useChartPlacement` (both reads via `dashboardService`, only when the
  graph has charts; any failure → `null` → ungrouped, no message), and a
  `GroupToggle` button node (`aria-expanded`, `BarChart3` as in the nav,
  `ChartColumn` for "Other charts"; the count never truncates).
  `asset-lineage.tsx` `GraphCard` shows its existing loading skeleton while
  the placement loads, so the map never draws 111 charts and then folds them.
- **T3** `src/lib/pan-zoom.ts` (pure: `fitScale`, `fitView`, `clampView`,
  `zoomAt`, `boxHeight`) and `PanZoomBox` in `asset-lineage-map.tsx`. No
  existing DOM pan/zoom helper (grep: `geo-view.ts` is ECharts' 1–20 roam
  for map tiles, a different model); no new dependency. One CSS transform on
  one wrapper; the drawing is `React.memo`'d so a pan does not redraw nodes.
  Safari trackpad pinch is handled through `gesturestart`/`gesturechange`
  in addition to Ctrl+wheel.

### Interpretations the reviewer should check

1. **Box height (decision 3, "keeps today's height rules").** Today the
   box is as tall as the drawing with no limit. Kept: the box is the
   drawing's height at the zoom that fits its width, **capped at 560 px**
   (`BOX_CAP`); without a cap a tall drawing (an opened 111-chart group)
   could never be "opened fitted". The width-fit shrink avoids an empty band
   under a wide map on a phone.
2. **Fit never enlarges.** The fit is capped at 1, so a small map opens at
   zoom 1 (as today) and cannot be zoomed out below 1.
3. **Opening or closing a group fits again** (eased), since the drawing's
   size changes and its new nodes may be off screen.
4. **Touch:** the box is `touch-action: none` so one finger pans the map
   (decision 3); on a phone the page scrolls from outside the box, which
   the 560 px cap leaves room for.
5. Group caption is "Dashboard", "Other charts" group caption "Charts".

### Checks run (final commit of T3/T4, foreground, every bun command under
`flock /tmp/claude-1000/lh-bun.lock systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`)

- `bun x tsc --noEmit -p .` → exit 0, no output.
- `bun x eslint` on the 10 changed `.ts`/`.tsx` files → exit 0, no output.
- `bun test src/features/dashboards/chart-lineage-panel.test.tsx src/features/catalog/asset-lineage-map.test.ts src/features/catalog/asset-lineage-mindmap.test.tsx src/features/catalog/asset-detail-tabs.test.tsx src/features/governance/used-by.test.tsx src/lib/pan-zoom.test.ts`
  → **210 pass, 0 fail, 689 expect() calls, 6 files**. Of these, the
  BI-71 files: panel 11, `asset-lineage-map.test.ts` 10 (4 new),
  `asset-lineage-mindmap.test.tsx` 6 (new), `pan-zoom.test.ts` 6 (new).
- The same checks ran per commit, scoped to that commit's files.

### Not verified

- **No browser check** (this VM: no dev server, no build): the look of the
  route and the map, the 240 ms draw, the pan/zoom feel, Safari's gesture
  events, a real phone pinch, and the hand check T3 asks for at 1190×800
  and 390 px. happy-dom lays nothing out, so the component tests stub the
  box width (`@/lib/test-layout`) and test the arithmetic, not pixels.
- **Tailwind output** of the new `@theme` animations (`animate-route-*`)
  is not verified by a build; they follow the existing `--animate-pop`
  pattern in the same block.
- Full `bun run test` / `bun run lint` not run (VM memory rule); CI runs
  them. `asset-detail-tabs.test.tsx` prints one more act() warning
  (`GraphCard`, from `useChartPlacement`'s resolved no-op read) among its
  many existing ones; it does not fail.
- No cargo (no Rust touched).

## Owner QA round 1 (2026-11-21)

1. **Source and dataset steps have no icon and a raw label.** The API sends
   step kinds `source` (a publisher, e.g. "Badan Pusat Statistik") and
   `dataset` (a catalog dataset); neither is in `STEP_KIND_LABEL`
   (`src/lib/lineage-words.ts`) nor in the panel's `KIND_ICON`, so they show
   a dot and "source"/"dataset" in lower case. Add both to both maps, using
   the mockup A icons (`Landmark` for a source, `Database` for a dataset;
   check `mockup-a.tsx` on `design/chart-builder-mockups`) and the labels
   "Source" and "Dataset". Grep the API (`rust/crates/lakehouse-api/src/routes/lineage*`)
   for every step kind it can send and cover any other missing one the same
   way; a test asserts every kind the API sends has a label and an icon.
2. **The map gets a full-screen view** ("biar lebih lebar luas lagi
   liatnya"). A fourth ghost icon button beside `+`, `−`, Fit (`Maximize2`,
   becoming `Minimize2`; tooltip "Full screen" / "Exit full screen", with
   `F`) opens the same map in an in-app overlay over the whole window
   (`fixed inset-0`, the app's background, the controls in the same
   corner), fitted on open and on close. Esc, the button, or `F` closes it;
   focus returns to the button. Not the browser Fullscreen API (iPhone
   Safari does not support it for elements). Opening and closing ease over
   200 ms, none under reduced motion. Page scroll is locked while it is open.

## Handoff 2 (owner QA round 1)

Commits:

- `6ba2b96db` step icons and labels: `source` (Landmark, "Source"), `dataset`
  (Database, "Dataset"), and `iceberg` (Table2, "Iceberg"), a third kind the
  API sends (`routes/lineage.rs`, `g.node(.., "iceberg")`) that had neither.
  `LINEAGE_STEP_KINDS` lists every kind the API sends; `src/lib/lineage-words.test.ts`
  holds both maps to it. The list is kept by hand, so a new API kind must be
  added there.
- `27d443c70` full-screen map: a fourth ghost button (Maximize2 / Minimize2,
  tooltip "Full screen" / "Exit full screen", `F`) opens the map in a portal
  overlay (`fixed inset-0 z-50`, `role="dialog"`), fitted on open and on close
  (the map inside is remounted each way). Esc, the button or `F` closes it;
  focus returns to the inline button; `body` scroll is locked while open;
  200 ms opacity fade, none under reduced motion. `stubRectWidth` in
  `src/lib/test-layout.ts` takes an optional height.

Commands (each wrapped in flock + systemd-run, 3G cap):

- `bun x tsc --noEmit -p .`: clean.
- `bun x eslint` on all changed files: clean (after replacing a ref read in render with state).
- `bun test` on the panel test, `asset-lineage-mindmap.test.tsx`,
  `asset-lineage-map.test.ts`, `pan-zoom.test.ts`, `lineage-words.test.ts`: 36 pass, 0 fail.

Not verified: no browser check (overlay look, fade, iPhone Safari, safe-area
insets); no focus trap inside the overlay (Tab can reach the page behind it);
an overlay opened inside another modal was not tried; the full test run and
CI were not run. Nothing pushed.

## Review of owner QA round 1 (planner, 2026-11-21)

- **SHOULD-FIX 1:** the full-screen overlay (`role="dialog"`) has no focus
  containment: Tab reaches the page behind it. Use the app's dialog
  primitive (or its focus trap) so Tab stays inside, as every other dialog
  does; keep Esc, `F` and the focus return. Fix with the next owner QA round.
