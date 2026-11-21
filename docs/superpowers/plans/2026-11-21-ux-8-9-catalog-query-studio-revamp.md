# `UX-8` and `UX-9`: Catalog, asset detail and Query Studio revamp

Planner and reviewer: Claude Opus. Developer: an **Opus design developer**
(console design work, `AGENTS.md` exception), separate from the planner.
Base: `main` (`d77e5f1b0`). Branches: `feat/ux-8-catalog-revamp` (PR 1 and
PR 2) and `feat/ux-9-query-studio-revamp` (PR 3, off `main`).

## Where this comes from

- The product owner asked on 2026-11-21 for a UI/UX revamp of the Catalog and
  Query Studio, after the 25 BI acceptance items.
- Mockups A/B/C of each screen were built on the local mockup branch
  (`design/chart-builder-mockups` at `a236793f3`, worktree
  `~/mtharreq/lakehouse-mockups`, code in `src/features/mockups/data-pages/`,
  routes `/mockups/catalog`, `/mockups/catalog-asset`, `/mockups/query-studio`).
- The owner's choices (2026-11-21):
  - **Catalog list:** mockup **A**, "but the data table is the advanced data
    table we already have".
  - **Asset detail:** mockup **A**.
  - **Query Studio:** layout **A**, plus the **bottom status bar of B**
    (engine · estimate · cache · data age · ⌘↵ run · ⌘⇧P commands), "except
    Ask stays a tab".
  - "Make sure no feature is lost."
  - Also approved: a primary **Add data** on the Catalog (opens Sources),
    **Certification** moves into the asset's ⋯ menu, and **Make a chart**
    from a Query Studio result.
- `DESIGN.md` (owner's taste) applies throughout; read it first. New rule from
  this choice: every list uses the one existing advanced data table.

## The no-loss rule (applies to every PR)

`docs/superpowers/plans/2026-11-21-ux-8-9-inventory.md` lists every
user-facing capability of the three screens today, with `file:line`, plus what
is duplicated and the gaps found. Each PR's handoff carries an **inventory
map**: one row per inventory item of its screen, saying where it lives now
(control and file), or "merged with #n" for a duplicate collapsed into one
place, or "removed (owner, date)" — and no removal without the owner's word.
The reviewer checks the map item by item and in the browser. An item without
a row is a `BLOCKER`.

## Shared decisions

1. **The page frame is the app's one frame** (`DESIGN.md` §3): title left;
   on the right, the icon-only search that widens to ~280 px (and `/`), view
   switch if any, ⋯, the primary action; all controls 32 px; one row on a
   laptop. No page description paragraph under the title.
2. **Lists use `src/components/data-table/data-table.tsx`** with its
   advanced toolbar (`data-table-advanced-toolbar.tsx`), as today. The
   revamp changes the frame and the cells around it, not the table
   component. Changes to the shared table itself (for all lists) are out of
   scope unless a decision below names one.
3. **Monospace only for SQL, code and identifiers shown as code** (a table
   address like `serving.mart_wisman` under a name is code). Labels, counts,
   badges and dates are sans with `tabular-nums`.
4. **Each fact once.** The inventory's "Duplicated" list is the worklist:
   collapse each duplicate to one place unless the second place does a
   different job (say which in the map).
5. **States are designed**: loading (one skeleton shaped like the page),
   empty, no match, error with Retry, permission denied, not found. "Not
   measured" is said once, with what to do about it.
6. **Tests**: update the screen's tests to the new markup; keep every
   behavioural test (filters, search, permissions, run/stop, downloads,
   time travel, saved/history) passing. New tests for new behaviour (Add
   data target, Make a chart, status bar, command menu).

## PR 1, `UX-8` part A: the Catalog list (mockup A)

- Frame: "Catalog" title; right side: search (icon, widens), ⋯ (Export CSV,
  Refresh, **Dropped tables · n** which opens today's dropped-tables list,
  Columns… for column visibility), and the primary **Add data** → the
  Sources page (`/connectors`, nav "Sources"; only when the person has
  `connector:manage`, otherwise no primary).
- One filter row above the table: the layer segmented control (All, Bronze,
  Silver, Gold) and a searchable **Namespace** picker as the quick filters,
  then the table toolbar's Filter, Sort and Reset. Layer and Namespace are
  filtered in one place only (drop them from the Filter menu or make the
  quick controls the menu's own state; pick one and say which).
- Name cell: name plus the table address under it (monospace, smaller);
  the separate Namespace column goes (its copy action stays in the row ⋯).
  Certification badge stays in the name cell.
- Freshness: "—" when not measured, with one ⓘ on the column header saying
  what "not measured" means. No per-row "Not measured" text.
- The width note ("Tags, Owner are hidden at this window width…") is
  removed; columns fold by the table's width as today, silently. The
  namespace summary card's facts (owner, residency, engine) appear once, as
  one line under the filter row when a namespace is picked.
- Keep: search over name, column, tag, description, owner with the matched
  field shown; every column filter; sort; row ⋯ and right-click menu (open,
  query, copy table name, copy ID, copy namespace); infinite loading and
  "Showing X of Y"; every page state.

## PR 2, `UX-8` part B: the asset detail (mockup A, with C's lineage strip)

- Header: name, certified mark, the address (copy), layer · tier ·
  classification · type in one line, one-sentence description; right: ⋯
  (Certification, Edit details, Publish to open format, Copy table name,
  Copy ID, Drop table — each with its current permission) and the primary
  **Query** (opens Query Studio with the starter query). Request access
  shows where access is missing, once.
- One strip of four trust facts (Updated, Quality, Used, Access), each a
  link to its tab; replaces the four Overview tiles and the facts strip.
- Tabs as today (Overview, Columns/Schema, Sample, Lineage, Quality,
  Access, Activity) with counts. Overview reads like a readme: the real
  columns with their value profile, the starter query (Copy, Run), Details
  (one list, merging the meta grid and About), and a short Lineage list.
  The Lineage tab opens with C's source-to-chart strip above today's map.
- Sensitive columns are marked inline in the column list and summarised in
  one line; no empty card. Connections and Recent changes appear once
  (Lineage tab, Activity tab), not as empty Overview cards.
- Keep every edit dialog, sensitivity per column and whole table, meaning
  view, schema versions, quality rules and schedules, access list and
  request, activity (usage, storage, change history, freshness target,
  query this version), and every state (sparse Bronze, no access, not
  found, loading).

## PR 3, `UX-9`: Query Studio (layout A + B's status bar)

- Frame: "Query Studio" title; right: the page sections Studio · Saved ·
  History (one control; the right-rail Saved and History cards are removed,
  their Load action lives in the Saved and History pages' row menus).
- Editor toolbar row: **SQL | Ask tabs** (Ask is the natural-language
  mode, a tab, not in the status bar), the query tab bar, Save (with Save as
  new and Save as SQL source in its menu), ⋯ (Format, Format settings,
  Keyboard shortcuts, Explain with AI, Time travel…), the side-panel
  toggle, and **Run**, which becomes **Stop · 2.4 s** in the same place
  while running.
- **Status bar** under the editor (mockup B): engine menu (an app menu, not
  a native select; engines named properly, e.g. "ClickHouse"), the estimate
  (scan · cost; "No pre-run estimate for Trino" when so), cache and data age,
  and the hints "⌘↵ run · ⌘⇧P commands". The estimate and engine are shown
  here only (not again in the toolbar or panel).
- **Command menu (⌘⇧P)** listing every editor action, including the ones in
  ⋯; it is an extra way in, never the only one.
- One right **side panel** with tabs **Schema | Plan** (the estimate's
  details and the execution plan), replacing the stacked Execution
  transparency card. Results keep their own run metrics once.
- **Time travel** is a ⋯ item; while a version is pinned it shows as a
  removable chip by the editor. Resolve the inventory's note (code allows
  Trino, a comment says it is disabled): follow the code, fix the comment.
- Results: table (the same data table), statement tabs for several
  statements, copy as TSV, Download (CSV, TSV, Excel, Parquet; ClickHouse
  only, as today), filter rows, cell inspector, column statistics, View in
  Audit, Fix with AI (diff to accept or reject), and the new **Make a chart**
  in the results ⋯: saves the run's SQL as a SQL source (today's
  save-source flow) and opens the chart builder on it; no new API.
- Keep `{{param}}` fields, query tab rename/close/+, autocomplete, the
  keyboard shortcuts dialog, Ask's agentic run and "Write SQL only", and all
  run states (empty, running, results, error, refused, stopped).

## Out of scope

- API changes (none are needed; Make a chart reuses existing routes).
- The shared data table's own features (density, keyboard row activation,
  full-query CSV export) — noted in the inventory as gaps, not part of this
  revamp.
- Other pages; the mobile layout beyond "readable, no sideways scroll".

## Checks (this VM)

Console only: wrap bun in `flock /tmp/claude-1000/lh-bun.lock systemd-run
--user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15 …`;
`bun x tsc --noEmit -p .`, `bun x eslint <changed>`, `bun test <touched
tests>`. No full test/lint, no prettier, no cargo. Look at each screen in
headless Chrome at 1280 and 390 wide, light and dark, every state, and
attach the screenshots' paths in the handoff. CHANGELOG lines. Do not push.

## Handoff

(developer appends per PR, with the inventory map)

### Handoff, PR 1 (`UX-8` part A, Catalog list) — Opus design developer, 2026-11-21

Branch `feat/ux-8-catalog-revamp`, not pushed.

**Commits**
- `c5e554c33` UX-8 feat(catalog): table pieces for the app's page frame
- `0eef31505` UX-8 feat(catalog): the Catalog list in the page frame (mockup A)

**Decisions taken where the plan said "pick one"**
- Layer and Namespace: **the quick controls are the Filter menu's own state** (option 2). Dropping the two columns from the menu is not possible without loss: `getFiltersStateParser` drops the *whole* `filters` param when an entry names a column that is not filterable, and the menu is the only place for Bronze (raw) alone, Bronze (curated) alone, or two namespaces. To keep each filter on screen once, a filter the switch or picker shows draws no chip, is not counted on the Filter button and does not open the property bar alone (opt-in `meta.isQuickFilter`, `isCatalogQuickFilter`); a filter they cannot show (raw only, Silver + Gold, namespace `iLike`) keeps its chip. With "or" or in the advanced rule list every rule shows.
- Bronze segment = `layer in [raw, bronze]` (the API sends only `raw`, `bronze`, `silver`, `gold`; `semantic` is never emitted). A segment is pressed only on an exact match.

**Changes to shared components (all opt-in, no other list changes)**: `useDataTableSearchValue` and `useDataTableReset` split out of their components; `DataTableSettingsMenu variant="page"` (⋯ trigger, "Columns" label, `extraItems`); toolbar `settingsMenu={false}`; `meta.isQuickFilter`; `EmptyState icon`; Dashboards' `SearchField` takes `inputLabel` / `buttonLabel` (defaults unchanged).

**Differs from the plan**
- Plan mismatch, stopped: the "Keep" list names row actions "query" and "copy table name", which do not exist today. **Copy table name** was added (it copies the address now shown under the name). **Query from a list row was not added**: list rows carry no `queryTarget`/`tableName`/`schema`, and the owner's rule (2026-11-08/09) is that a Query Studio link may only name a table the page has proven readable; a correct link needs the asset detail read first. Needs a planner decision (fetch the detail on select, or drop it).
- Name cell keeps click-to-copy of the name (inventory 28); the address under it is plain, copied from the row menu.
- Columns still fold by **window** width (as today); the note naming them is gone.
- Search and ⋯ hide when there is no list (403, error, unsupported); Add data stays.
- No "18 assets" count in the filter row (the footer says "Showing X of Y"; each fact once).

**Commands (this VM, wrapped in `flock … systemd-run … MemoryMax=3G … nice -n 15`)**
- `bun x tsc --noEmit -p .` — clean (0 errors).
- `bun x eslint <20 changed files>` — clean.
- `bun test src/features/catalog/catalog-page.test.tsx src/features/catalog/dropped-tables.test.tsx src/features/catalog/data-explorer-columns.test.tsx src/features/dashboards/dashboard-list-page.test.tsx src/components/data-table/data-table-add-filter.test.tsx src/hooks/use-data-table-filters.test.tsx src/components/patterns/page-states.test.tsx` — 80 pass, 0 fail.
- Not run (forbidden here): full `bun run test`, `bun run lint`, `bun run build`.

**Screenshots** (`/home/hv/mtharreq/lakehouse-ux8/.ux8-shots/`, not committed; real QA API as `qa-analyst`, states faked with CDP request interception where the stack has no such data): `catalog-a-{light,dark}-{1280,390}`, `catalog-search-light-1280`, `catalog-nomatch-{light,dark}-1280`, `catalog-nomatch-light-390`, `catalog-namespace-{light,dark}-1280`, `catalog-picker-{light,dark}-1280`, `catalog-filter-light-1280`, `catalog-menu-{light,dark}-1280` (Add data via an added `connector:manage`), `catalog-adddata-light-390`, `catalog-dropped-light-1280` (faked list), `catalog-loading-{light,dark}-1280`, `catalog-error-{light,dark}-1280`, `catalog-denied-{light,dark}-1280`, `catalog-empty-{light,dark}-1280`.

**Not verified**: a real `connector:manage` login (permission added by interception), real dropped tables (none on the QA stack), an iPhone, the app's own auth placeholder before the page (a different skeleton, pre-existing, not this page's).

**Inventory map, section 1 (Catalog list)**

| # | Where it lives now |
|---|---|
| 1 | Title "Catalog" in the page frame (`catalog-page.tsx` header). Description paragraph removed (plan decision 1). |
| 2 | ⋯ → "Dropped tables · n" (shown when n > 0 or the read failed) → dialog, same rows, error + Retry (`dropped-tables.tsx`). |
| 3 | `CatalogTableSkeleton` under the real frame and filter row. |
| 4 | `ErrorState` with Retry (403 → permission notice), unchanged. |
| 5 | Unsupported-tenant `EmptyState`, unchanged. |
| 6 | Designed: "Nothing in the catalog yet" (`CatalogNoRows`). |
| 7 | Route-level states, unchanged. |
| 8 | Removed by the plan (note gone; columns still fold). |
| 9, 10 | Unchanged. |
| 11 | Page search: icon-only `SearchField` (`/`, Escape) writing the same `search` param (`CatalogSearch`). |
| 12 | Matched-on line under name and address, unchanged. |
| 13 | Searchable Namespace picker (`NamespacePicker`): All, each namespace with id and count; loading placeholder ("Loading namespaces"); error + Retry inside the picker; hidden with zero namespaces. |
| 14 | One line under the filter row (`NamespaceLine`): name · description · owner · residency · engine. "Assets" count merged with the footer; "All namespaces" merged with the picker's first item. |
| 15 | Layer switch All · Bronze · Silver · Gold (`LayerSwitch`); Raw and curated are one Bronze; each apart stays in Filter menu → Layer. |
| 16–23 | Table toolbar Sort, Filter, property bar, advanced rules, variants, operators, Reset — unchanged (quick filters draw no chip, see above). |
| 24a | ⋯ → Columns (was Settings → View). |
| 24b, c | ⋯ → Reorder, Group. |
| 24d, e, f | ⋯ → Export CSV (count), Refresh data, Reset layout. |
| 25–27 | Unchanged. |
| 28 | Name cell: name (click-to-copy), certification badge, address under it (mono), matched-on line. |
| 29 | Namespace column hidden by default; still in Columns, Filter, Group. |
| 30–34, 36 | Unchanged (Layer cell gains its dot). |
| 35 | Freshness: "—" when not measured, one ⓘ on the header (`FRESHNESS_NOT_MEASURED`). |
| 37, 38 | Unchanged (hidden by default). |
| 39 | Row ⋯, unchanged. |
| 40–43 | Row ⋯ and right-click: Open asset, **Copy table name** (new), Copy asset ID, Copy namespace. |
| 44–52 | Unchanged. |

### Handoff, PR 2 (`UX-8` part B, asset detail) — Opus design developer, 2026-11-21

Branch `feat/ux-8-catalog-revamp`, not pushed.

**Commits**
- `3fde397d3` UX-8 fix(catalog): no sideways scroll in the dropped-tables dialog (PR 1 follow-up, found in the screenshots)
- `92f017f8e` UX-8 feat(catalog): the asset page as a readme (mockup A, with C's lineage strip)

**Commands (same wrapper as PR 1)**
- `bun x tsc --noEmit -p .` — clean.
- `bun x eslint <15 changed files>` — clean.
- `bun test src/features/catalog/` — 452 pass, 0 fail, 20 files (covers both PRs' catalog tests).
- Tests changed to the new markup, none weakened: the tab is "Columns", Edit details / Certification / Drop table open from the header ⋯ (`renderPage`, `openAssetMenu`), the About card is the Details region, storage tests open the Activity tab, the Overview's starter query is a region, health reasons are the header badge's title. One assertion ("no day count", DATA-16 D2) is scoped to the Snapshots card it is about, because the Storage card now shares the tab. New: `asset-header.test.ts` (trust facts) and the `UX-8` block in `asset-detail-tabs.test.tsx` (Query primary, strip, short lineage list, Overview columns).

**Screenshots** (`/home/hv/mtharreq/lakehouse-ux8/.ux8-shots/`, not committed; QA API as `qa-analyst`): `asset-a-{light,dark}-{1280,390}` (Gold `serving.mart_wisman`, 390 full page), `asset-sparse-{light,dark}-1280` and `asset-sparse-light-390` (Bronze `wisman-bulanan`, no readable table, unprofiled), `asset-lineage-{light,dark}-1280` (Silver, strip + map), `asset-columns-light-1280`, `asset-activity-light-1280` (full page, storage at the end), `asset-menu-{light,dark}-1280` (⋯ with `governance:write` and `catalog:write` added by interception), `asset-publish-light-1280`, `asset-noaccess-light-1280` (session faked to `catalog:read` only: Request access, no Query), `asset-loading-{light,dark}-1280`, `asset-notfound-{light,dark}-1280`, `asset-denied-{light,dark}-1280` (403 faked).

**Not verified**: Drop table in the ⋯ in a browser (no upload-made table on the QA stack; covered by tests), Certification / Edit details saves in a browser (tests cover the calls), a real `gold:export` holder, an iPhone. The Overview's "Open in Query Studio" still shows without `query:read` (unchanged from before; the header's Query does check it) — flag for the owner whether to align them. The asset route's navbar breadcrumb shows the workspace name (pre-existing, untouched).

**Machine notes**: the dev server ran on :28630 with Turbopack against a hard-linked copy of `lakehouse-preview/node_modules` (`cp -al`, no extra disk): Turbopack refuses a `node_modules` symlink that points outside the project root, and webpack dev was OOM-killed at the 3 GB cap. The copy is git-ignored (`/node_modules`) and nothing of it is staged. While locating the QA login I printed one QA user's line of `qa-users.env` (its password) into my own tool output; it is in no file, commit or message, but the planner may want to rotate that QA password.

**Differs from the plan**
- Health stays a header badge, with what it rests on as its title (the strip holds the plan's four facts; the Overview's Health tile is merged into that badge).
- Request access stays one header button where access is missing; the Sample/Lineage/Access empty states keep pointing at it.
- Publish in open format opens the existing card in a dialog from ⋯ (no card on the Overview any more); the card itself still gates the switch and Publish now on `gold:export`.
- Drop table moved from the Snapshots card to ⋯ only (restore stays on the Snapshots card).
- Storage (Iceberg and ClickHouse) moved to the end of the Activity tab.
- The tab label "Schema" reads "Columns"; `?tab=schema` is unchanged. Lineage moved after Sample (mockup order).
- The Overview's starter query keeps "Open in Query Studio" (mockup says "Run"); the header primary is **Query**.
- A "Catalog" back link stays above the name: the navbar breadcrumb names the workspace, not the Catalog, on this route (pre-existing).
- Mockup A's "Latest" changes on the Overview are not drawn: the plan puts changes on Activity only.

**Inventory map, section 2 (asset detail)**

| # | Where it lives now |
|---|---|
| 1 | `AssetPageSkeleton` (header, strip, tabs, readme + side). |
| 2 | `ErrorState`, unchanged. |
| 3 | "‹ Catalog" link above the name (`asset-header.tsx`). |
| 4, 5 | Header h1 + certification badge. |
| 6, 7, 8 | Header line two: tier, classification, health (titles unchanged). |
| 9 | ⋯ → Certification… (`governance:write`) → the same dialog (`CertificationDialog`). |
| 10 | Header line two, Meaning badge. |
| 11 | Header button Request access + its dialog, unchanged. |
| 12 | Description under the header (two lines, full text as title). |
| 13 | Deprecation notice under the description, unchanged. |
| 14 | Merged with the address (header line two, copy button). |
| 15, 16 | Header line two (type; layer with dot and title). |
| 17, 18 | Details → "Stored in" (engine · format). |
| 19 | Details → Rows ("—" with title when the Iceberg table is gone). |
| 20, 21, 22 | Details → Size, Owner, Residency. |
| 23, 24 | Trust strip → Updated (relative, "· late" when past target) → Activity tab. |
| 25 | Tabs with counts: Overview, Columns, Sample, Lineage, Quality, Access, Activity. |
| 26 | `?tab=`, unchanged values. |
| 27 | Merged with the header Health badge (#8). |
| 28 | Merged with trust strip Updated; snapshot count stays on Activity. |
| 29 | Trust strip → Quality ("p of e passed · n failed / warnings", "No checks", "n rules, not run yet") → Quality tab. |
| 30 | Trust strip → Access ("You can read · n masked") → Access tab; classification in the header. |
| 31 | Overview → Details (`AssetDetails`). |
| 32 | Marks inline in the Overview's column rows and the Columns tab; one summary line (`sensitiveLine`). Empty card removed. |
| 33 | Activity tab, end (`AssetStorage` / `ClickHouseStorage`). |
| 34 | ⋯ → Publish in open format… (Gold) → dialog with `OpenFormatCard`. |
| 35 | Overview → "Start a query": SQL, Copy, Open in Query Studio, engine sentence, `NO_READABLE_TABLE`; also the header's Query. |
| 36 | Overview → Lineage short list (comes from / read by, link to the tab) + Lineage tab; table-grant note kept. |
| 37 | Activity tab → Change history (not on the Overview, plan). |
| 38 | ⋯ → Edit details… (`catalog:write`) → the same dialog (`EditDetailsDialog`). |
| 39 | Details list (Domain, Owner, Steward, Update frequency, Unit, Publisher classification, Schema, Tags). "Columns" count merged with the Columns tab count. |
| 40 | Details → Sensitivity (whole table), unchanged control. |
| 41 | Edit details dialog, unchanged. |
| 42–47 | Columns tab, unchanged. |
| 48–57 | Sample tab, unchanged. |
| 58–63 | Quality tab, unchanged. |
| 64–69 | Access tab, unchanged. |
| 70–80 | Lineage tab, unchanged, with the new strip above (`LineageStrip`). |
| 81–88 | Activity tab, unchanged. |
| 89 | ⋯ → Drop table… (same permission and server checks: `canDropTable`, `useDropTable`, same dialog). |
| 90–99 | Same card, in the Publish dialog (#34). |
| 100, 101 | Activity tab (#33). |
| 102 | Unchanged. |
