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

(developer)
