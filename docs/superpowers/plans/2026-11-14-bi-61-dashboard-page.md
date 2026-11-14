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
9. **T9, the route loading fallback draws the page's own placeholder (owner QA 2026-11-15).**
   Owner: "ini pas aku ketiga halaman ini skeleton yang list itu masih ada… pas logout juga ada
   tu skeletonnya". `src/app/loading.tsx` draws the generic `LoadingSkeleton rows={8}` for every
   route while a segment loads (visible on a slow network on every navigation), so Home and a
   dashboard show the generic list first and then their own placeholder, and `/login` after
   sign-out shows the list full width. Fix:
   - Lift the main-area switch of `AppShellSkeleton` (`app-shell-skeleton.tsx`, the
     `placeholder === …` branch) into one exported component (e.g. `ShellMainPlaceholder`
     taking `pathname`), used by both `AppShellSkeleton` and `src/app/loading.tsx`. One source,
     so the auth check, the route fallback and the page's own loading look are the same shapes.
   - `loading.tsx` becomes a client component: on a public path (`isPublicPath`, e.g. `/login`,
     the landing, public dashboards, embed, render) it draws nothing on the page background (no
     list); inside the app it draws `ShellMainPlaceholder` for `usePathname()`, with the same
     outer padding/frame as the real main area so nothing jumps when the page replaces it.
   - Check: unit tests that `loading.tsx` draws the dashboard placeholder on `/dashboards/<id>`,
     the list placeholder on `/dashboards/browse`, nothing on `/login`, and the generic one on
     another page; the shell skeleton tests still pass.
10. **T10, the Dashboards row is named after the dashboard it opens (owner QA 2026-11-15).**
   Owner: "kalo emang pas di klik bukan ke halaman browse nama menunya diubah jadi nama dashboard
   yang dibuka dong". The row's label part goes to `/dashboards`, which opens the board last
   opened (`dashboard-resolver.tsx`), so its label says which board that is:
   - `last-board.ts` remembers the board's name with its id (`rememberLastBoard(id, name)`), and
     the dashboard page passes the name when it opens a board and again when the board is
     renamed. Reading an old id-only value still works (no name: the label stays "Dashboards").
     `forgetLastBoard` clears both. Changes are announced in this tab (a custom event) and in
     other tabs (`storage`), so the sidebar updates at once.
   - The sidebar's Dashboards row shows that name (truncated with an ellipsis, full name in the
     collapsed-rail tooltip) with the same chart icon; "Dashboards" when nothing is remembered.
     The right part (Browse) is unchanged. The row's `aria-label` keeps the word: "Dashboard:
     <name>" so screen readers know what the row is. The navbar title and the breadcrumb are
     not changed.
   - Server-render and first paint use "Dashboards" (localStorage is not readable there); the
     name swaps in after mount without moving the row (same height, same icon slot).
   - Check: unit tests for the label with and without a remembered board, an old id-only value,
     forget, and the update when another board is opened.
   - **Browse resets it** (owner, 2026-11-15: "kalo misalnya user pencet browse dashboard. berarti
     menu Dashboardnya kembali lagi dan kalo user klik halaman dashboard dia ke browse"). Opening
     `/dashboards/browse` (by the right part or any other way) forgets the last board, so the
     row reads "Dashboards" again. With nothing remembered, the row's label part links straight
     to `/dashboards/browse`; with a board remembered it links to that board
     (`/dashboards/<id>`), so the row never depends on the resolver's guess. Opening a board
     remembers it again. Check: a test that visiting Browse clears the name and the row's href
     becomes `/dashboards/browse`; opening a board sets both back.
11. **T11, the row remembers dashboards and reports, in its own memory (owner QA 2026-11-15).**
   Owner: "berlaku buat halaman dashboard dan reports ini juga. pokoknya halaman yang ada di
   dashboards nih yang namanya kesimpan di menu". T10 stored the row's name in the last-board
   memory, which also drives the `/dashboards` resolver and Home's focal dashboard, so Browse's
   reset made Home lose its last board. Split the two:
   - **Last board** (`last-board.ts`, id plus name): back to its pre-T10 meaning for the resolver
     and Home. Opening Browse no longer forgets it. Deleting a board still does.
   - **The row's memory** (new, e.g. `dashboards-row.ts`): `{kind: "dashboard" | "report", id,
     name}`, the last dashboard or report opened. Set when a dashboard page or a report page
     (view or edit, `/dashboards/reports/<id>…`) opens, and again on rename. Cleared when Browse
     opens and when that item is deleted (the page's delete, Browse's bulk delete and trash).
     Same tab and cross-tab updates and the same `useSyncExternalStore` hook pattern as T10.
   - The row shows that item's name, and its icon is the item kind's icon, the same one Browse
     uses for a dashboard or a report. Its label part links to `/dashboards/<id>` or
     `/dashboards/reports/<id>`. With nothing remembered: "Dashboards", the row's own icon, and
     a link to `/dashboards/browse`. `aria-label` "Dashboard: <name>" or "Report: <name>".
   - Metrics (`/dashboards/metrics`) and templates are not remembered: they are not items one
     reopens.
   - Check: tests for a report setting the row (name, icon, href), a dashboard after a report
     replacing it, Browse clearing the row but **not** the last board, deleting the remembered
     item clearing it, and Home's focal board still following the last board after a Browse
     visit.

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

### Developer handoff, review round 1 (2026-11-14)

**Commits:**

| Fix | Commit |
| --- | --- |
| SHOULD-FIX 1–6: fade at cut edges, no sparkles, one-line tile note, short edit hint, chips at the left without tabs, Auto-refresh value beside its chevron | `2300963d` |
| SHOULD-FIX 7: the account menu in the sidebar's footer | `a32f0f56` |
| SHOULD-FIX 8–10: centred navbar search, 32 px header controls, Edit · settings · Share | `40108851` |
| SHOULD-FIX 11: one time in the header; "Edited" in Details | `6ce96a10` |

**Notes:**

- **SHOULD-FIX 2:** the mark beside "jumlah by tahun" was the clickable-chart hint (`MousePointerClick`), not an AI mark. It is a pointing hand now. An AI-made chart gets a muted `Bot` mark with "Made with the assistant". The side panel's "AI Copilot" tab and the empty state use `Bot`. The Copilot's own sidebar header (other pages, not this branch) still has its `Sparkles`.
- **SHOULD-FIX 1:** `useScrollFade` plus `.scroll-fade` (globals.css) fade an edge only while it hides something, over 150 ms, instantly under reduced motion. Used on the dashboard tile area (lg and up) and its frame (below lg); no other page on this branch scrolls inside a frame.
- **SHOULD-FIX 7:**
  - `UserMenu` moved to `user-menu.tsx` and takes a `trigger`.
  - The menu opens upward from the footer, aligned to its start.
  - **Not done:** `side="right"` when the sidebar is collapsed, because `DropdownMenuContent` forwards only `align` and `sideOffset` and the base component was not changed.
- **SHOULD-FIX 8:** the search keeps the navbar's existing 36 px height (correction in SHOULD-FIX 9).

**Checks** (under the 3 GB cap, one at a time):

- `bun test`, one file per run, 58 files (the T7 set plus `details-panel`, `use-scroll-fade`): 470 pass, 0 fail.
- eslint on `src/features/dashboards`, `src/features/copilot`, `src/components/app-shell` and the new hook: 0 errors, 1 warning (`use-copilot.ts`, not from this branch).
- `bun run typecheck`: clean.

**Not verified:**

- No round-1 screenshots. Port 28622 was held by another developer, then 28620 ran a Next dev server, and the owner's rule is one Next dev server at a time. The coordinator screenshots from 28605 after deploy.
- The fade, the footer menu (collapsed and in the phone sheet), the centred search and the 32 px header were checked by tests only, not in a browser.

### T9 handoff, 2026-11-15 (Sonnet developer agent)

**Commit** `71b81eb7` (`fix(console): BI-61 T9, the route loading fallback draws the page's own placeholder`; not pushed).

- New `src/components/app-shell/shell-main-placeholder.tsx`: `ShellMainPlaceholder({ pathname })`, the main-area switch lifted out of `AppShellSkeleton`; it draws no padding.
- `app-shell-skeleton.tsx` uses it inside its existing `p-4 sm:p-5 lg:p-6` wrapper.
- `src/app/loading.tsx` is a client component: `null` on `isPublicPath`, else `ShellMainPlaceholder`. No wrapper padding, because it renders inside `AuthenticatedFrame`'s `<main>`, which already pads (the same box as the skeleton's wrapper).
- New `src/app/loading.test.tsx`: `/dashboards/b1`, `/dashboards/browse`, `/login` (empty), `/alerts` (generic).

**Checks** (each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`, one at a time, after `free -m` showed over 3 GB available): `bun run typecheck` clean; `bun test src/components/app-shell src/app/loading.test.tsx` 58 pass, 0 fail (10 files); `bun x eslint` on the 4 changed files clean.

**Not verified:** a real browser on a slow network (no dev server run); the full test and lint runs. The skeleton's wrapper has no `pb-24` as `<main>` has when the Copilot dock shows, as before this change.

### T10 handoff, 2026-11-15 (Sonnet developer agent)

**Commit** `9b47f01b` (`feat(console): BI-61 T10, the Dashboards row is named after the dashboard it opens`; not pushed). Includes the "Browse resets it" bullet.

- `last-board.ts`: the name is stored in a separate key `dashboards:last-board-name` as `{ id, name }`, so the id key, `readLastBoard` and its readers (resolver, Home) are unchanged and an old id-only value still works. The name is read only when its id matches the remembered id. `rememberLastBoard(id, name?)`, `forgetLastBoard` (clears both) announce through a window event (this tab) and `storage` (other tabs). New hooks `useLastBoardName` and `useLastBoardId` (`useSyncExternalStore`, server snapshot `null`).
- `dashboard-page.tsx`: a new effect after `knownName` remembers `(board, knownName)`, so it also runs after a rename.
- `dashboard-list-page.tsx`: mount calls `forgetLastBoard()`.
- `app-sidebar.tsx`: only the `/dashboards` row changes: label = name, `aria-label` "Dashboard: <name>", tooltip = name, href = `/dashboards/<id>` when an id is remembered, else `/dashboards/browse`. Same icon and slot; the name truncates.
- Tests (`app-sidebar.test.tsx`, 5 new): no board, remembered name, old id-only, forget (what Browse does), another board / rename / name of another id.

**Checks:** `bun run typecheck` clean; `bun test src/components/app-shell src/features/dashboards` 789 pass, 0 fail; eslint on the changed files clean.

**Not verified:** no browser check; the Browse-mount effect in `DashboardListPage` has no test of its own (the sidebar test covers the forget).

## Micro-interaction audit (emil), 2026-11-14

Owner request (2026-11-14): audit the revamped pages' micro-animations with the
`emil-design-eng` checklist. DESIGN.md wins over the skill: 150–250 ms, eased,
never linear; motion with a reason; everything stops under reduced motion.
Opus developer agent (console design exception). Motion only: no layout,
colour or copy changes; roles and accessible names unchanged.

Shared part (commit `feat(console): motion foundation`, cherry-picked to BI-59 and WEB-17 so the three branches carry the same lines):

| Element | Now | Skill / DESIGN.md | Change |
| --- | --- | --- | --- |
| Easing tokens | Tailwind `ease-out` (0,0,0.2,1) and three hand-written curves | Strong custom ease-out; ease-in-out for movement | `ease-ui`, `ease-move`, `ease-drawer` theme tokens |
| Reduced motion | One `motion-reduce:` on the sidebar, nothing elsewhere | §3: every movement stops | One global rule in `globals.css` |
| Tooltips | Keyframes (`animate-in`), open at once (provider delay 0), same speed in and out | Delay the first, then instant (`data-instant`); transitions, origin-aware, exit faster | Transition 150 ms in / 100 ms out from `--transform-origin`, `data-instant` → 0 ms, root delay 350 ms |
| `Button` press | `active:translate-y-px`, `transition-all` | `scale(0.97)` on `:active`; never `transition: all` | `active:scale-[0.97]`, named properties, `ease-ui` |
| Menus / popovers | 150 ms `ease-out`, scale 0.95 both ways; popover 100 ms default ease | Strong ease-out, exit faster than enter | 150 ms in / 100 ms out, `ease-ui`, scale 0.97 |
| Dialog / sheet | 200 ms `ease-out`; sheet 300 ms `ease-in-out` both ways | Modal stays centred; drawer curve; exit faster | Dialog 200/150 ms `ease-ui`; sheet 300/200 ms `ease-drawer` |
| Transition lists naming `transform` | `active:scale-*`, `rotate-*` did not animate (Tailwind v4 sets `scale`/`rotate`/`translate`) | Name the property that changes | Lists name `scale`, `rotate`, `translate` |

Dashboard page and shell (this branch):

| Element | Now | Skill / DESIGN.md | Change |
| --- | --- | --- | --- |
| `EASE` (header, chips, tiles, Details) | Lists `transform`: press scale and chevron turn jumped | Name the property | `scale,rotate,translate`, `ease-ui` |
| Edit / settings / Share press | `active:scale-95` / `0.98`, not animated | 0.95–0.98, animated | Base 0.97, animated |
| Star (favourite) | Changes only after the server and the shortcut reload answer | Feedback at once (§3 "no button sits still") | Optimistic, reverts on failure, `animate-pop` when it turns on |
| Dashboard tabs | One underline per tab, fading out/in | Motion that shows where the choice went | One underline that slides (`useSlidingIndicator`, 240 ms `ease-move`), press 0.97 |
| Filter chips, + Filter, Reset, Parameters | No press | Press 0.97 | `active:scale-[0.97]`; the whole chip gives under its value or × |
| Chip choice tick | Blinks on | Grow from 0.75, never from 0 | Scale 0.75 → 1 with opacity |
| Edit banner | Pops in | Enter eased, exit fast | `@starting-style` fade down 200 ms; leaving edit mode drops it at once |
| Edit mode grid lines, resize corners | Pop in; corner hover not eased | Prevent jarring changes | Fade in; corner grows from 0.9, eases hover, 1.1 on press |
| Dragged tile | Ring only | Drag should feel held | Lifts (scale 1.01, soft wide shadow), settles on drop |
| Tiles pushed by a drag | Tailwind default 150 ms curve | Strong ease-out | 200 ms `ease-ui` |
| Tile data landing | Placeholder replaced in one frame | Fill in place, no blink | Body content fades in over the same box (placeholder excluded, so skeleton → grid does not blink) |
| Title loading → name | Bar replaced in one frame | Same | Name fades in on its line |
| Side panel (Copilot, Comments, Details) | Appears in one frame | Drawer from its edge, exit fast | Slide 16 px + fade, 260 ms `ease-drawer`; closing instant |
| Side panel tabs | Fill jumps between tabs; content swaps in one frame | Sliding indicator; crossfade | Sliding pill (`box`), press 0.97, content fades in |
| Panel / dock icon buttons | Colour only | Press feedback | Press 0.95, 150 ms |
| Bottom dock chat | Appears in one frame; `transition-all` | Origin-aware, named properties | Rises from the bar (origin bottom), `transition-[width]` |
| AI Copilot navbar button | `transform` listed: press jumped | Name the property | `scale`, `ease-ui` |
| Navbar search | No hover | Hover response | Background eases on hover |
| ⌘K palette | No open animation | Keyboard, many times a day: none | Kept as is (deliberate) |
| Sidebar section chevron | `rotate-90` snapped (`transform` listed) | Name the property | Turns 200 ms with the fold |
| Sidebar icon-mode flyout | Appears in one frame | Grows from its trigger | Grows from the rail (origin left), 150 ms |
| Sidebar rows | Colour 150 ms | Rows used tens of times a day: keep small | Kept; `ease-ui` on the sub-rows |
