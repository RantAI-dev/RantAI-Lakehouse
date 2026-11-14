# BI-61: the dashboard page (focused reader), plan

| | |
| --- | --- |
| Backlog | `BI-61` |
| Spec | `docs/core/specs/bi-61.md` |
| Feature page | `docs/core/features/dashboard-reader.md` |
| Base | `feat/bi-18d-reports-templates` at `0662957c` (PR #528). This PR follows #528 |
| Branch | `feat/bi-61-dashboard-page` |
| Planner and reviewer | Claude Opus (hub session) |
| Developer | an Opus developer agent (console design work, AGENTS.md exception) |

## 1. Decisions already made

- **Layout A (owner, 2026-11-14).**
  - The reference is commit `c80a6172` on the local branch `design/chart-builder-mockups`: its
    `src/features/mockups/dashboard-view/` (mockup A) and its screenshots in
    `/home/hv/mtharreq/lakehouse-mockups/.mockup-shots/dashboard-view/a-*.png` and `x-a-*`.
  - Read it with `git show c80a6172:<path>`.
  - Copy the look and the structure, not the fixture.
- **One side panel at a time (DESIGN.md).**
  - Comments and Details (sharing summary, freshness, version history, subscriptions) open in
    the same right side panel as the AI Copilot, as tabs. Opening one replaces what the panel
    shows; two panels never show side by side.
  - Find how the Copilot panel is mounted in the app shell, and add the dashboard's tabs to it
    instead of a second panel.
  - If that needs an app-shell change wider than this page, stop and report.
- **The composer reserve.** BI-58 (`feat/bi-58-chart-builder-revamp`, `c4d87211` and
  `50295df6`) exposes `--content-bottom`. This branch does not have it yet, so read it with a
  fallback, as BI-59 did in `library-parts.tsx`. Do not copy BI-58 code.
- **All existing features stay reachable:**
  - tabs, filters and parameters;
  - drill (the click menu);
  - each tile's actions;
  - comments, subscriptions, sharing, PDF, print, exports, duplicate, rename, delete, theme,
    freshness, auto-refresh, version history, save as template;
  - the phone layout and dark mode.
- **Out of scope:**
  - mockup B's workspace panel, and mockup C's report mode;
  - new features, and the API.
- **Assistant parity:** no tool change, and the page context is unchanged.

## 2. Anchors (at `0662957c`)

- **Route:** `src/app/(overview)/dashboards/[id]/page.tsx` renders `DashboardPage` from
  `src/features/dashboards/dashboard-page.tsx` (1,629 lines).
  - The per-tile "Manual" badge comes from the refresh-mode table at `:143`.
  - "Parameters" is at `:1364`.
- **`dashboard-actions.tsx` (301 lines):**
  - the ⋯ trigger, an outline button `aria-label="More actions"`, at `:82`;
  - "Edit layout" / "Done" at `:281–288`;
  - "New chart" at `:293–298`.
- **Tiles:** `tile-card.tsx`, `tile-body.tsx`, `tile-dialogs.tsx`, `tile-drills.tsx`.
- **The grid:** `dashboard-grid.tsx`.
- **The board switcher:** `board-switcher.tsx`, which A replaces with a breadcrumb.
- **Filters and parameters:** `filter-row.tsx`, `filters/`, `param-controls.tsx`,
  `param-dialog.tsx`.
- **Panels and dialogs:** `comments-panel.tsx`, `share-dialog.tsx`, `subscriptions-dialog.tsx`,
  `freshness-dialog.tsx`, `auto-refresh.ts`.
- **Tests:** keep every existing test green. Update selectors where things move; never weaken
  an assertion.

## 3. Tasks (one commit each, in order)

1. **T1, header.**
   - Build: the breadcrumb, the title, the star, the facts line (owner · updated · data from ·
     N comments, where the comments link opens the Comments tab) and, on the right, Edit
     (pencil, for writers), Share (primary) and ⋯.
   - Keep the console's shadcn components, restyled to our tokens: no raw defaults, 36 px
     controls (DESIGN.md).
   - Check: spec T1.
2. **T2, the ⋯ menu.** Group it into View, Get a copy and Manage, with auto-refresh as one
   submenu.
   - Check: every old entry is reachable, and a test lists them.
3. **T3, tabs and filters row.**
   - Tabs and filter or parameter chips share one quiet row under the header.
   - Each chip opens a small panel with a visible label and full-width controls.
   - A changed chip is tinted, and one Reset shows when anything differs.
   - "+ Filter" appears in edit mode only.
4. **T4, tiles.**
   - Soft fill, no border, no badge.
   - On hover, show the ⋯ and a "Data from …" line.
   - The tile ⋯ holds every current tile action plus the refresh mode.
   - Drill stays as today.
5. **T5, edit mode.**
   - The pencil switches to edit: Add chart (primary), Done, drag handles, "+ Filter", "+ tab".
   - The current layout editing keeps working.
6. **T6, one side panel.**
   - Comments and Details become tabs of the right panel the AI Copilot uses.
   - Check: opening Comments while the Copilot is open swaps the panel's content, and the tiles
     reflow once.
7. **T7, responsive, the composer, and screenshots.**
   - The phone layout stacks, and tabs and filters work there.
   - The page frame reads the composer reserve as in §1.
   - Screenshots at 1190×800, 1366×900 and 390×844, light and dark, reading and editing, plus
     the ⋯ menu and the Comments panel, into `docs/plans/bi-61-screens/`. Check them yourself.
8. **T8, handoff.**

## 4. Machine rules (owner, 2026-11-14)

- **No Rust.**
- **Tests:** never the full `bun run test`. Run scoped files only, one command at a time, each
  with `nice -n 15` under
  `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0`, after checking `free -m`
  shows at least 3 GB available.
- **Screenshots:** a brief dev server from this worktree on **28622**, only when it is free
  (another developer may be using it), plus one headless Chrome with its own profile. Stop both
  by PID right away.
- **Port 28620** is the owner's combined preview. Never touch it.

## 5. Review

(planner)

## 6. Handoff

### Developer handoff, 2026-11-14 (Opus developer agent)

**Commits** (branch `feat/bi-61-dashboard-page`, on `ba76c5a1`; not pushed):

| Task | Commit | What |
| --- | --- | --- |
| T1 | `9b2e07e6` | Header: breadcrumb, title with star and Private badge, facts line, Edit pencil, Share, ⋯ |
| T2 | `e3480344` | ⋯ menu in View, Get a copy and Manage; Auto-refresh and Export as submenus |
| T3 | `cdf5f5fd` | Tabs and borderless chips on one row; tinted when changed; one Reset |
| T4 | `a0fbb014` | Soft tiles with no border and no badge; hover ⋯ and "Data from"; tile ⋯ reordered |
| T5 | `607eac44` | Edit mode: Add chart and Done, the edit hint line, Parameters and "+ Filter" in the row |
| T6 | `e5cabaca` | Comments and Details as tabs of the Copilot's side panel |
| T7 | `7abd685e` | Frame above the composer, phone, screenshots |

**Checks** (each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`, one at a time, after `free -m` showed at least 3 GB available):

- `bun test <file>`, one file per run, 56 files: 462 pass, 0 fail. The files:
  - `src/features/dashboards`: `dashboard-*`, `tile-*`, `filters/*`, `filter-row`, `param-controls`, `grain-switch`, `tab-bar`, `comments-panel`, `board-switcher`, `embed-view`, `render-view`, `template-pages`, `drill`, `use-tile-drills`, `table-tiles`, `switched-tile`, `public-map-boundary`, `share-dialog`, `subscriptions-dialog`, `version-history-dialog`, `freshness-dialog`, `theme-font-dialog`, `download-dialogs`, `chart-lineage-panel`, `chart-alert-dialog`;
  - `src/components/app-shell/*`, `src/features/copilot/*`, `src/lib/page-context-summary.test.ts`.
- One file at a time because several files mock `./echart` with `mock.module`: run together in one `bun test`, they break each other (seen on this branch with `tile-freshness`, `tile-limits` and `render-view`; each passes alone).
- `bun --bun eslint src/features/dashboards/ src/features/copilot/ src/components/app-shell/`: 0 errors, 1 warning (`use-copilot.ts`, `changedABoard` unused, not from this branch).
- `bun run typecheck`: clean.
- **Not verified:** the full `bun run test` and `bun run lint` (machine rules: CI runs them). No Rust, Python or compose was touched.

**New tests:** `dashboard-header.test.tsx` (header controls per role, breadcrumb and facts, edit mode in and out, the frame), `tile-card.test.tsx` (soft look, menu notes, Download submenu), `side-panel.test.tsx` (one panel, tab swaps, the Copilot replacing a page tab, the bottom dock stepping aside, no Copilot tab without the assistant, unchanged without page tabs), and new cases in `dashboard-actions`, `filter-bar`, `param-controls` and `grain-switch`.

**Tests changed where things moved** (no assertion weakened):

- the parameter, grouping and refused-parameter tests open the chip before reaching its control;
- the interval and export tests open their submenu;
- the Share tests moved from the menu to `ShareButton`;
- "Add filter" is found by its button name;
- `dashboard-comments-param` renders the shell's side panel and closes it with "Close panel";
- `filter-row.test.tsx` now asserts one sideways-scrolling line at 390 px instead of the "Filters · N" sheet (see choice 6).

**Screenshots** in `docs/plans/bi-61-screens/` (QA stack, board "QA BI" unless named), all checked:

- `reading-` and `editing-` × `light`/`dark` × `1190x800`, `1366x900`, `390x844`;
- `menu-light-1366x900`, `menu-auto-refresh-dark-1366x900`, `tile-menu-light-1366x900`, `chip-light-1366x900`;
- `comments-copilot-light-1366x900` (Copilot docked right, Comments tab chosen), `comments-light-1190x800`, `comments-dark-390x844`, `details-dark-1366x900`;
- `reading-light-1366x900-template` ("QA dari template").

Measured: frame bottom / composer top 720/738 px at 1190x800, 820/838 at 1366x900, 764/782 at 390x844; 876 of 900 with the side panel open; no window scroll and no horizontal scroll anywhere. A brief dev server on 28622 and one headless Chrome (own profile in the scratchpad) were stopped by PID after each session; 28620 and the QA stack were not touched; `AGENTS.md` (rewritten by `next dev`) was reverted each time.

**Choices the plan did not settle:**

1. **The side panel (T6) needed an app-shell change, confined to pages that opt in.** The Copilot's right panel is `CopilotSidebar`, mounted by `AppFrame` only for holders of the assistant. A new `components/app-shell/side-panel.tsx` lets a page register tabs and open one; `CopilotSidebar` then draws a tab row (AI Copilot, Comments, Details, one Close) and the page portals the tab's content into the panel body. `AppFrame` adds the provider and now mounts the sidebar for everyone (it draws nothing until something opens), so a reader without the assistant gets Comments and Details alone. `CopilotDock` hides its bottom bar while a page tab is open, and a page tab opening folds the bottom dock's rising chat; choosing the "AI Copilot" tab docks the Copilot right. A page that registers no tabs sees the shell as before (tested). With tabs, the panel covers the page below `md`. I judged this not wider than the page, since only the dashboard page registers tabs, but it does touch `app-frame.tsx`, `copilot-sidebar.tsx` and `copilot-dock.tsx`; the reviewer may decide otherwise.
2. **Details opens from the facts line** ("ⓘ Details" after the comments), so the header keeps three controls and the menu 14 entries. It reads only existing services: who can open it and the people added (managers), public link, embed, data from, auto-refresh, freshness, subscriptions (first four), versions (last five), each with its one entry into the existing dialog.
3. **The "Manual" badge was the chart's source, not a refresh mode** (§2's anchor calls it "the refresh-mode table"; it is `SOURCE_BADGE`: `ui` → "Manual", `ai` → "AI"). The tile ⋯ now says both: the dashboard's auto-refresh ("auto-refresh off" / "refreshes every 5m") and the source as words ("made by hand" / "made with AI"), with the data time and what it reads.
4. **"+ Filter" for readers.** T3 puts it in edit mode, but a reader who cannot edit has no edit mode, and adding a temporary filter was theirs before (§1, everything stays reachable). It stays in their reading row; writers find it in edit mode.
5. **Parameters** (the old header button, `dashboard:write`) is in the filter row in edit mode. Someone with the global write permission but only view level on this dashboard no longer reaches it from here; the chart builder still opens the same dialog.
6. **Phone filters:** the chips stay on one line that scrolls sideways (mockup A's phone) instead of the "Filters · N" sheet; `filter-row.test.tsx` was rewritten for that.
7. **Board switcher:** replaced by the breadcrumb, as A does; "New dashboard" from the switcher went with it (the list page has New). `board-switcher.tsx` and its test stay, now unused by the page.
8. **Tile look is opt-in** (`DashboardGrid look="soft"`): the embed and the public link keep their bordered cards, badges and "Data from" footer.
9. **Edit mode:** Done is outline and Add chart primary, per the plan (mockup A had them the other way round). The phone-layout and heading/section controls moved into the edit hint line. The tiles' ⋯ shows without hover in edit mode and on touch screens.
10. **List parameters and the date grouping** are columns of ticked choices in the chip's panel, not selects: a select's popup sits outside the panel and closes it on the first click.

**Not done or not verified:**

- No QA board has tabs, so the tab row is shown only as "+ Add tabs" in edit mode. The tab styling is covered by `tab-bar.test.tsx` but was not seen with real tabs.
- The hover-only ⋯ and "Data from" line rely on `pointer-fine`. Headless Chrome reports no fine pointer, so the screenshots show the ⋯ on every tile.
- On a phone, the facts line can wrap so that a "·" starts the second line.
- `--content-bottom` itself was not exercised: this branch has no BI-58. Only the fallbacks were measured.
