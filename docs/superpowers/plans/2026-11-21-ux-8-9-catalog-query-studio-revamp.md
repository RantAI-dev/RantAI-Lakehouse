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
