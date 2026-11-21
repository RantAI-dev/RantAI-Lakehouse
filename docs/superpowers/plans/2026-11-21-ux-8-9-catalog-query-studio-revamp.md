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

### PR 3 (UX-9): Query Studio, layout A + B's status bar

Developer: Opus design developer (console design exception), branch
`feat/ux-9-query-studio-revamp` off `main` (`17bd04216`). Not pushed.

**Commits**

1. `56350614a` read and remove a version pin from the SQL (`pinnedSnapshots`,
   `unpinSnapshots` in `src/lib/snapshot-picker.ts`; the SETTINGS item split
   is one helper shared with `pinSnapshot`).
2. `02901748f` sections on the title row (Studio · Saved · History, one 32 px
   segmented control on all three pages), and a row menu on History (Open in
   Studio, View in Audit).
3. `7615f8f60` the result's header row (metrics line, Run details popover,
   Filter rows, Copy as TSV, Download icon menu, ⋯ with Column statistics,
   View in Audit, Make a chart).
4. `7445d9677` Save as SQL source with `next: "chart"` goes on to the chart
   builder (Make a chart).
5. `f39e4a101` layout A with B's status bar and the ⌘⇧P command menu (the
   page, toolbar, status bar, side panel, command menu, time-travel dialog
   and chip, format settings dialog; the Saved/History rails removed).
6. this commit: CHANGELOG (Unreleased → Changed) and this handoff.

**Commands run (this VM, wrapped in the bun lock and the 3 GB scope)**

- `bun x tsc --noEmit -p .` on the final commit: exit 0, no output.
- `bun x eslint <the 37 .ts/.tsx files added or changed since 17bd04216>`:
  exit 0, no output.
- `bun test <the 12 changed test files> src/features/queries/` on the final
  commit: **481 pass, 0 fail, 29 files** (all of `src/features/queries/` plus
  `src/lib/snapshot-picker.test.ts`).
- Commit 4 (`7445d9677`, before the layout commit) checked in a throw-away
  worktree: `tsc` exit 0; `bun test src/features/queries/` 440 pass, 0 fail,
  28 files. Commits 2 and 3 are subsets of it and were not checked on their
  own.
- Not run (VM rules): `bun run test`, `bun run lint`, `bun run build`, cargo
  (no Rust touched), prettier.

**Screenshots**: not taken (no dev server, planner checks on QA). The planner
asked for this on 2026-11-21: the one dev server slot was the UX-8
developer's. States to look at, 1280 and 390, light and dark: empty ("Results
show here"), running (Stop · time, the moving bar), results, error (Retry,
Fix with AI on an `invalid_request`), refused ("You don't have access"),
stopped ("Run again"), Ask, time travel pinned (chip), command menu
(⌘/Ctrl+Shift+P), side panel Schema and Plan. Each of these states is
rendered by a test in `query-studio-page.test.tsx`,
`query-studio-stop.test.tsx`, `query-results-section.test.tsx` or
`query-transparency-panel.test.tsx`.

**What differs from the plan (none of it changes a decision; say if one should)**

- The query tab bar is its own row above the toolbar row, as in mockup A;
  the plan's sentence lists it inside the toolbar row. With up to 20 tabs it
  needs a row that scrolls.
- The status bar sits directly under the editor, inside the editor's box (as
  the plan says), not at the bottom of the workbench as in mockup B.
- Save on a new tab is labelled **Save** (it was "Save as new"); **Save as
  new…** is in Save's menu only when Save changes a saved query, so a new
  tab has one save action (`DESIGN.md` §3). Save as new writes the save
  draft with `mode: "create"` and the saved query's id, which the Save page
  already treats as a copy of that query.
- Run reads **Run** / **Run all** (was "Run query"); while running it is
  **Stop · 2.4 s** (the time `aria-hidden`, with an `sr-only` `role=timer`
  "Elapsed time" beside it).
- The toolbar's **Estimate** button and the panel's Retry were one action
  twice (inventory duplicate): Retry stays on an error (status bar and Plan
  tab), and **Estimate again** is at the Plan tab's foot after a ready
  estimate and in the command menu.
- The Plan tab does not repeat the engine, Est. scan or Est. cost (the plan:
  "the estimate and engine are shown here only"). Cache and data age are in
  the status bar only when measured, and in the Plan tab's details always
  (Cache "Not measured" when null), so "not measured" is said once.
- **Make a chart** opens the builder of the built-in dashboard
  (`/dashboards/default/charts/new`, draft `{ sqlSource }`); the builder's
  own "Add to dashboard" picker moves it. It is offered only with
  `dashboard:sql` (to write the source) and `builtInBoardEditable` (to reach
  the builder); without the second the builder would show "no access".
- The section label "Saved Queries" became **Saved** (mockup), and the
  History page got a row menu: the plan places the rails' Load in "the Saved
  and History pages' row menus", and History had none.
- The schema browser lost its "Schema" heading and close button: the Schema
  tab names it and the panel toggle closes it (one action, one button).

**Not verified**

- Anything in a browser (layout at 1280/390, dark mode, the sticky panel,
  focus rings, motion). No screenshots.
- ⌘/Ctrl+Shift+P on Firefox (Ctrl+Shift+P there opens a private window;
  `preventDefault` may not win). Chrome and Safari bind nothing to it.
- `DESIGN.md` "one side panel at a time": the AI Copilot sidebar and the
  Schema | Plan panel can both be open. Nothing in the plan covers it; I
  left the two independent. **Needs the planner/owner.**
- Generate SQL (Write SQL only) switches to the SQL tab (`loadSql`), so its
  Explanation card in the Ask tab is only seen on going back. This was so
  before (inventory note 13); not changed.

**Review follow-up (planner decisions, 2026-11-21)**: Schema | Plan and the
AI Copilot panel share one slot (the shell's `useCopilot` docked-right +
expanded state; opening either closes the other, tested both ways). F1 opens
the command menu too and is in the shortcuts dialog; the status bar names the
platform's keys (no helper existed, so `isApplePlatform`/`studioKeys` were
added to `src/lib/editor-shortcuts.ts`). This supersedes the two matching
"Not verified" items above.

**Browser review fixes (planner, QA stack, 2026-11-21)**, one commit:
1. Schema rows: the cause was the console's base `li { font-size: 16px }`
   (globals.css; the same rule sized bare `<p>`s in the Plan tab). Every row
   now names its size (13 px, normal weight) and has one shape in every layer:
   the name alone, truncated, with the full name and the address it inserts
   (or why it cannot) in the row's tooltip.
2. Result cells: scoped to Query Studio (`query-results-section.tsx` builds
   its own columns over the shared `DataTable`; `sql-rows-table`/`rows-table`
   are the dashboard and pipeline previews, not used here). A column is
   numeric by the statistics' own rule (`isNumericColumn`), so ClickHouse's
   quoted 64-bit sums count; numbers go through the dashboards' formatter
   (`lib/cell-format.ts`) in the Settings → Reporting number locale
   (`GET /api/settings/reporting`, any signed-in person), numeric text gets
   the same separators with every digit kept (`formatDecimalText`, BigInt),
   and numeric columns sort by value. Cells are sans 13 px, numbers
   `tabular-nums` and right-aligned; the inspector button shows on row hover
   or focus only. Copy as TSV, filters and the inspector still read the raw
   value.
3. Autocomplete: CodeMirror's emoji icons are off (`icons: false` in Query
   Studio's completion extension) and each option ends with a small muted
   word (column, table, parameter, keyword); the list's `li` reads at 13 px.
   The dashboard SQL source and pipeline editors use the editor's basic setup
   and keep the default icons: not changed here.
4. Default mode: Ask (natural language), as before the revamp — the hook
   starts on `"nl"` (`use-query-studio.ts`, unchanged) and a stored draft
   restores the person's last choice; links that carry SQL (`?sql=`,
   `?saved=`, `?run=`) open on SQL. Not changed.

**Inventory map (section 3)** — "kept" means the same behaviour, labels and
tests, restyled; files under `src/features/queries/` unless stated.

| # | Where it lives now |
|---|---|
| 1 | Studio · Saved · History, one segmented control on the right of the title row of all three pages (`query-studio-tabs.tsx`); label "Saved Queries" → "Saved" |
| 2 | Title "Query Studio" kept (`query-studio-page.tsx`); description removed (shared decision 1) |
| 3 | `?saved=` alert kept, under the title row |
| 4 | `?run=` alert kept, under the title row |
| 5 | SQL \| Ask tabs in the toolbar row (`ModeTabs`, `studio-toolbar.tsx`); "Natural language" → "Ask"; also "Switch to Ask/SQL" in the command menu |
| 6 | Unchanged (`studio.tab`) |
| 7 | Ask tab textarea (`nl-panel.tsx`); label `sr-only` (the Ask tab is its visible name), `aria-required` kept |
| 8 | **Ask** (primary; "Agent working…"), plus ⌘↵ in the textarea |
| 9 | **Write SQL only** ("Writing…") |
| 10 | One line: "Ask runs the query and explains it · Write SQL only fills the editor" |
| 11 | Kept |
| 12 | Kept (`agent-result-card.tsx` unchanged) |
| 13 | Kept (see Not verified) |
| 14 | Kept, one row above the toolbar that scrolls sideways (`query-tab-bar.tsx`) |
| 15 | Kept (pencil, quiet until the tab is open/hovered/focused) + double-click on the name |
| 16 | Kept |
| 17 | Kept ("Add query tab", limit message) + "New query tab" in the command menu |
| 18 | Unchanged |
| 19 | `sr-only` "SQL" label; editor keeps `aria-labelledby` and `required` (`sql-panel.tsx`) |
| 20–23 | Unchanged |
| 24 | ⋯ → Keyboard shortcuts, and the command menu; the dialog also lists ⌘/Ctrl+Shift+P (`keyboard-shortcuts-dialog.tsx`, now controlled) |
| 25 | Merged with the status bar hint "⌘↵ run · ⌘⇧P commands" and the ⌘↵ key on Run |
| 26 | Kept, inside the editor box between the editor and the status bar |
| 27–31 | Unchanged (`query-parameter-*`) |
| 32 | Status bar "Fill in {{x}} for an estimate"; the Plan tab keeps the full sentence |
| 33 | Status bar engine menu, an app menu with ClickHouse and Trino (`studio-status-bar.tsx`); "Run on …" in the command menu |
| 34 | **Run** / **Run all** (`RunButton`); "Running…" is the result area's "Running on X…" |
| 35 | Unchanged (alert under the editor) |
| 36 | The Run button's place while a run or download is going, same titles; "Stop" in the command menu |
| 37 | In the Stop label + `sr-only` `role=timer` "Elapsed time" |
| 38 | **Save** (update or create as before); **Save as new…** in Save's menu when Save updates; "Select one statement" kept |
| 39 | Save's menu (perm `dashboard:sql`) + command menu |
| 40 | ⋯ → Format + command menu |
| 41 | ⋯ → Format settings… → dialog with labelled Keyword case and Indent (`format-settings-dialog.tsx`) |
| 42 | See 24 |
| 43 | Side panel toggle "Show/Hide schema and plan" (`PanelToggle`); open by default at ≥ 1024 px; "Show schema" in the command menu |
| 44 | Merged with Retry: **Estimate again** at the Plan tab's foot and in the command menu, same disabled reasons |
| 45 | ⋯ → Explain with AI ("Explaining…") + command menu |
| 46 | Kept as the items' titles |
| 47 | Schema tab of the side panel (`studio-side-panel.tsx`); heading and close button removed (the tab names it, the toggle closes it) |
| 48 | Kept ("Search tables", now `type=search`) |
| 49–52 | Kept (`schema-browser.tsx` restyled; every label, reason and Retry unchanged) |
| 53 | ⋯ / command menu → Time travel… → dialog with labelled Namespace, Table, Version and "Query this version" (`time-travel-dialog.tsx`, `time-travel-controls.tsx`); new: a pin chip by the editor, × reads the current version (`unpinSnapshots`); new: "The query does not read X" on Trino instead of nothing |
| 54 | Kept in the dialog |
| 55 | Both engines, as the code did; the stale page comment went with the old layout, the controls' doc says so |
| 56 | Unchanged |
| 57 | Kept, under the editor above the result |
| 58–60 | Kept |
| 61 | Kept (`query-result-tabs.tsx`) |
| 62a | Copy as TSV, an icon button in the result's header row (same name, toast and disabling) |
| 62b | Download, an icon menu (same four formats) |
| 62c | Downloads-off sentence as a line under the header; "ClickHouse only" as Download's title |
| 62d | Result ⋯ → View in Audit |
| 63 | Unchanged; its Stop is the Run button's place |
| 64 | Kept |
| 65 | Behind **Filter rows** in the header row; hiding the fields clears them |
| 66 | Result ⋯ → Column statistics (toggles the panel) |
| 67–68 | Unchanged (same data table, cell inspector) |
| 69 | One metrics line (rows, duration, scanned, cost, engine; workload and cache when known); Pushdowns and Policy obligations in the **Run details** popover |
| 70 | Run details popover |
| 71–77 | Kept; running shows a moving bar and "Running on X…" (`role=status`); stopped adds **Run again**; new empty state "Results show here" |
| 78 | Removed: the Plan tab names it |
| 79 | Each state: a short sentence in the status bar and the full one in the Plan tab (Trino, empty, waiting, error + Retry, estimating, no estimate, cannot be planned) |
| 80 | Est. scan, Est. cost, Engine: status bar only; Workload, Cache, Freshness, Policy obligations, Sources (marks, governed tags, deprecation): Plan tab |
| 81 | Plan tab |
| 82 | Unchanged (debounce), Retry in the status bar and the Plan tab |
| 83 | Removed; merged with the Saved page (section control) and its row menu "Open in Studio" (`?saved=`, opens with the query's parameters, in a new tab rather than the open one) |
| 84 | Removed; merged with the History page (row click and row menu "Open in Studio", `?run=`) and the new row menu's "View in Audit" |
| 85 | Removed with the rails |
| 86–95 | Saved page unchanged; the section control moved into its header's actions |
| 96–109 | History page unchanged; the section control in its header's actions, and a row menu as the last column |
| 110–117 | Save page unchanged (Save as new arrives as `mode: "create"` with the source id) |
| 118–121 | Save as SQL source unchanged; with `next: "chart"` it is titled Make a chart and opens the chart builder on the saved source |
| 122 | Unchanged (Copilot page context) |
| 123–126 | Unchanged |
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
| 40–43 | Row ⋯ and right-click: Open asset, **Copy table name** (new), Copy asset ID, Copy namespace. The plan's "query" row action was a mockup item, not a current capability; not added (planner, 2026-11-21): the asset page's Query primary covers it, and a list row cannot prove the table readable. |
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

**Not verified**: Drop table in the ⋯ in a browser (no upload-made table on the QA stack; covered by tests), Certification / Edit details saves in a browser (tests cover the calls), a real `gold:export` holder, an iPhone. The Overview's "Open in Query Studio" now needs `query:read`, like the header's Query (planner review, 2026-11-21; the SQL and Copy stay for everyone). The asset route's navbar breadcrumb shows the workspace name (pre-existing, untouched).

**Review fix** (planner, 2026-11-21): `UX-8 fix(catalog): Open in Query Studio needs query:read, like the Query button (review)` — tested both ways in `asset-detail-tabs.test.tsx`; `bun test` on the three asset test files 194 pass, 0 fail; `tsc` and `eslint` (2 files) clean. The `node_modules` hard-link copy stays for now (planner removes it).

**Browser review fixes** (planner, 2026-11-21; `UX-8 fix(catalog): browser review`): (1) a mark ("sort key", "masked", …) never squeezes a column name: the name's line wraps one line tall, so a mark that does not fit drops whole (the opened column lists all marks) and the name truncates only when the column is narrower than it, with the full name as title — applies to the Overview preview and the Columns tab; (2) "No column statistics yet" with the server's reason in an ⓘ (Overview and Columns tab), so a Bronze asset with no readable table says "no readable table" once, under Start a query; (3) the Internal badge uses the soft sky fill (was primary/10, hard to read on dark). `bun test src/features/catalog/ src/components/patterns/status-badge.test.tsx` 459 pass, 0 fail; `tsc` and `eslint` (4 files) clean. One test pinned the old `@lg:flex` class on the marks and now pins `@lg:contents`, with the reason. Not looked at in a browser (no dev server for this fix).

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
