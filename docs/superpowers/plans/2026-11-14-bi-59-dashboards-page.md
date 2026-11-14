# BI-59: the Dashboards page (home and table, with a gallery view), plan

| | |
| --- | --- |
| Backlog | `BI-59` |
| Spec | `docs/core/specs/bi-59.md` |
| Feature page | `docs/core/features/dashboards-page.md` |
| Base | `feat/bi-18d-reports-templates` at `0662957c` (PR #528). This PR follows #528 and is rebased on `main` once #528 merges |
| Branch | `feat/bi-59-dashboards-page` |
| Planner and reviewer | Claude Opus (hub session) |
| Developer | an Opus developer agent (console design work, AGENTS.md exception) |

## 1. Decisions already made

- **Layout C (owner, 2026-11-14).**
  - The reference is commit `df0f38bb` on the local branch `design/chart-builder-mockups`:
    `src/features/mockups/dashboards/` (mockup C) and its screenshots in
    `/home/hv/mtharreq/lakehouse-mockups/.mockup-shots/dashboards/c-*.png`.
  - The same object store, so `git show df0f38bb:<path>` works here.
  - Copy the layout and the components' look, not the fixture data.
- **Gallery view (owner, 2026-11-14):** B's picture cards (`b-*.png`) become the Gallery view of
  the same list. Table is the default, and the choice is kept in `localStorage`, wrapped in
  try/catch so the page works without it.
- **Pictures (P3):** drawn from chart definitions, never from a query.
  - Read `GET /api/dashboard/specs` once for the whole page; it already returns every stored
    chart the person can see, with `board` and `def`. The console client is in
    `src/services/clients/dashboards.ts` (`getChart` reads it at `:231`). Add a list method
    there rather than calling `apiFetch` from the feature.
  - Group the charts by board and draw each tile as a small silhouette of its kind, in its
    span, in the console's chart colours, with sample shapes and no numbers.
  - Main (the built-in board) and a board with no charts get a neutral picture.
  - Reports draw their first chart blocks. Templates draw their stored definition's charts,
    which `list_templates` / the template list already carries; if it does not, a neutral
    picture.
  - Real-number pictures are `BI-60`, not this PR.
- **One list:** dashboards, reports and templates, with a segmented control
  **All / Dashboards / Reports / Templates** that replaces the tabs, plus a folder picker.
- **Header:**
  - one **New** menu: Dashboard, Report, From a template (each with its picture), Import;
  - one page ⋯ menu: Metrics, Manage folders, Trash;
  - a large search ("/" focuses it).
  - Remove the toolbar row of 11 controls and the Favourites and Recent chip rows.
- **Recent:** the four items opened last, as small picture cards, from what the page already
  reads for its Recent strip today.
- **Badges, one style** (P4, DESIGN.md "Badge tanpa border"): built-in grey, public link
  amber, embedded sky, restricted rose. Soft fill, no border.
- **Item ⋯ menu:** the same position on every row and card, Main included. Main's entries
  that cannot apply are shown disabled with the reason, as in the mockup.
- **No "No description."** Show the description when there is one; otherwise a useful fact
  ("Made from the template X", a report's first sentence) or nothing.
- **Bulk:** tick rows to Move to folder, Favourite, or Move to trash, through the existing
  services. No new API.
- **The assistant bar** covers no row. The list scrolls inside a frame that ends above the
  bar, as the mockup does; the real page needs the same approach.
- **Out of scope:**
  - the dashboard page itself;
  - sharing, folder access rules, trash behaviour;
  - mockup A's rail;
  - real-number pictures (`BI-60`).
- **Assistant parity:** no tool change. The list reads the same routes, and the assistant
  already has `list_boards`, `list_reports` and `list_templates`.

## 2. Anchors (at `0662957c`)

- **Route:** `src/app/(overview)/dashboards/browse/page.tsx` renders `DashboardListPage`.
- **`src/features/dashboards/dashboard-list-page.tsx` (837 lines):**
  - the "No description." fallback at `:277`;
  - Favourites and Recent at `:356` and `:601–602`;
  - the toolbar:
    - New dashboard at `:534`;
    - the Gallery and Table toggles at `:549` and `:565`;
    - Metrics at `:611`, Trash at `:621`, Import… at `:630`;
  - `TrashDialog` at `:797`.
- **Columns:** `dashboard-list-columns.tsx`.
- **Reports and templates lists:** `report-list.tsx`, `template-list.tsx`, `template-dialogs.tsx`.
- **Folders:** `folder-dialogs.tsx`, `folder-options.tsx`.
- **Tests to keep green**, updating selectors where things move and never weakening an
  assertion: every `*.test.tsx` that renders these files, including
  `folder-options.test.tsx` and `template-pages.test.tsx`.

## 3. Tasks (one commit each, in order)

1. **T1, header and page frame.**
   - Build the title, search, New menu and page ⋯ menu, and the scroll frame that ends above
     the assistant bar.
   - Remove the old toolbar row and the chip rows.
   - Check: spec T1, so no other button is visible.
2. **T2, one list.**
   - Build one table of dashboards, reports and templates with the columns name, type,
     folder, owner, updated and sharing; every column sorts; plus the segmented type filter
     and the folder picker.
   - Badges per P4. The item ⋯ menu sits at the row end, the same for every row.
   - Folder grouping, the restricted lock and personal space keep working.
   - Check: each filter shows only its kind; sorting and the folder filter combine.
3. **T3, pictures.** The specs list method, the silhouette renderer, and the Recent row.
   - Check: a test proves the page makes one specs request and no dashboard read for the
     pictures.
4. **T4, Gallery view.**
   - The view switch (Table / Gallery), remembered.
   - Picture cards grouped by folder, folders collapsible, as in mockup B.
   - Check: the choice survives a reload, and the page still renders if `localStorage`
     throws.
5. **T5, bulk and facts.**
   - Bulk Move to folder, Favourite, Move to trash; restore from the trash still works.
   - The useful fact replaces "No description.".
   - Designed empty states: no reports, no templates, an empty folder, no search match, an
     empty workspace.
6. **T6, responsive and screenshots.**
   - At phone width the table becomes a stacked list.
   - Screenshots at 1190×800, 1366×900 and 390×844, light and dark, Table and Gallery, into
     `docs/plans/bi-59-screens/`. Check them yourself.
7. **T7, handoff:** "Handoff, BI-59" below, with the commands and counts.

## 4. Machine rules (owner, 2026-11-14)

- **No Rust:** no Rust compiles; this PR touches no Rust.
- **Tests:** never the full `bun run test`. Run the touched test files only, under a memory cap:
  `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15 bun test <files>`.
- **Typecheck and lint:** `bun run typecheck` and `bun run lint` with `nice`, one at a time,
  only when `free -m` shows at least 3 GB available.
- **Dev server for screenshots:** at most one, briefly, on port 28622 if it is free. Stop it
  by PID afterwards. Do not touch port 28620 (the mockup server) or the QA stack.

## 5. Review

(planner)

## 6. Handoff

### Handoff, BI-59 (developer, Opus, 2026-11-14)

**Commits** on `feat/bi-59-dashboards-page`, over `d1aa6a87`, not pushed:

| Task | Commit | What |
| --- | --- | --- |
| T1 | `fcdc8885` | Header: title, New menu, page ⋯ menu, large search ("/" and Esc); the page frame that ends above the assistant bar; toolbar and chip rows removed |
| T2 | `dd4eeed6` | One sortable list of dashboards, reports and templates; segmented type filter in `?tab=`; folder picker; P4 badges; the same ⋯ menu on every row, Main included |
| T3 | `7cd31f6d` | `listChartOutlines` (one `GET /api/dashboard/specs`), the silhouette pictures, the Recent row |
| T4 | `85fd8a49` | Table / Gallery switch, remembered per browser; gallery grouped by folder |
| T5 | `35a20b1a` | Bulk Move to folder, Favourite, Move to trash; facts instead of "No description."; designed empty states |
| T6 | `1d4b4392` | Stacked rows below 768 px; screenshots |
| T6 follow-up | `77e42271` | DESIGN.md §3 rules added on 2026-11-14: one control per action (no second "Clear search"), a visible label in the move dialog |

**Commands and counts** (each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`):

- `bun test src/features/dashboards src/services/clients/dashboards.test.ts`, at `1d4b4392`: 781 pass, 0 fail, 91 files.
- `bun test dashboard-list-page.test.tsx template-pages.test.tsx library-item-menu.test.tsx`, at `77e42271`: 36 pass, 0 fail.
- `bun run typecheck`, at `77e42271`: clean.
- `bun --bun eslint src/features/dashboards/`, at `77e42271`: clean. `src/services/` at `7cd31f6d`: one warning that was already there (`http.ts:106`).
- **Not verified:**
  - the full `bun run lint`: killed by the OOM killer (exit 137) on the shared VM, so only the scoped eslint above ran. Left to CI.
  - the full `bun run test`: not run on the VM (machine rule). Left to CI.
- No Rust touched, no cargo run.

**Screenshots** in `docs/plans/bi-59-screens/`, taken against the QA stack's API with a dev server on 28622 and one headless Chrome, both stopped by PID afterwards:

- Table and Gallery, light and dark, at 1190x800, 1366x900 and 390x844: `{table,gallery}-{light,dark}-{1190x800,1366x900,390x844}.png`, 12 shots. No horizontal scroll at any size, and the assistant bar covers no row.
- States:
  - `state-bulk-light-1366x900.png`;
  - `state-new-menu-light-1366x900.png`;
  - `state-main-menu-dark-1366x900.png`: Main's disabled entries with the reason;
  - `state-page-menu-light-1190x800.png`;
  - `state-degraded-reads-light-1366x900.png`: the QA API dropped some reads during that shot. It shows the degraded page: glyph pictures, and "could not be listed" notes for reports and templates.
- Headless Chrome reports `hover: none`, so the outline stars that a laptop shows only on hover are visible in every shot.
- `next dev` rewrites `AGENTS.md`. The change was reverted and not committed.

**Choices the plan did not settle**

1. **No data in the lists for some of what the plan names.** These items use the plan's own fallback (a neutral picture, or nothing) and need the planner's decision:
   - **Report pictures.** `ReportSummary` carries no blocks, so a report cannot be drawn from its first chart blocks without one read per report. It is drawn as a page with its real title, and one grey block per chart, two at most.
   - **The report's "first sentence" fact.** Not available, for the same reason.
   - **"Made from the template X".** Boards do not record their source template.
   - **Template pictures.** The template list carries no definition, so a template gets a glyph and its chart count.
   - **The rose "Restricted" badge.** Whether a folder is restricted is only known per item (`GET /api/dashboard/access`). The tone is defined but never shown. The personal space keeps its lock and "Private" badge.
2. **Bulk actions are for writers only.** A reader has no tick boxes. Favourite skips reports, because the favourites API takes only `board` and `chart`, and the toast says so.
3. **The bulk move previews access.** `MoveItemsDialog` calls `previewMove` for each dashboard it moves (feature B7). It does not offer a personal space while a report is among the items. A single report's "Move to folder" uses the same dialog. A single dashboard keeps `MoveBoardDialog`.
4. **The item ⋯ menu** holds Open, Favourite (dashboards only), Copy link, Rename / Edit, Duplicate, Move to folder and Move to trash. "Sharing and embed" and "Save as template" were left out:
   - sharing is out of scope;
   - the list does not know the download switch that Save as template follows (BI-18 D review, BLOCKER 1).
5. **The view key is new.** The view choice uses a new key, `dashboards:browse-view`. The old page's key defaulted to Gallery, so reusing it would open some browsers on a view the owner did not choose.
6. **The Recent cards say "Dashboard · N tiles" instead of "opened N min ago".** The shortcuts list carries no time.
7. **The phone layout is chosen in script.** The page picks the stacked rows from the viewport (`useIsMobile`) instead of rendering both layouts and hiding one with CSS, so a screen reader does not read the rows twice.
8. **DESIGN.md "Satu aksi, satu tombol".** The no-match state lost its "Clear search" button, which duplicated the visible ✕ in the search box. Two empty-state buttons stay, because they are shortcuts to entries inside menus:
   - "Show all folders" (the folder picker);
   - "New report" (New).
   The star and "Add to favourites" in the ⋯ menu also both stay: the star is hidden on hover devices until hover. The planner may want either pair cut.
9. **The app-shell changes on BI-58** (`8f625135`: a 14rem sidebar; `c4d87211`: the composer centred on the content area, with a shared bottom reserve token). This branch does not have them and does not copy them.
   - `PageFrame` assumes nothing about where the composer sits horizontally.
   - Its height does hard-code the app frame's current bottom reserve: `pb-24` / `lg:pb-28`, 5rem for the bar.
   - When BI-58 merges, `PageFrame` (`library-parts.tsx`) should read the shared reserve token instead.


### Handoff, BI-59 review round 1 (developer, 2026-11-14)

**SHOULD-FIX 1: search and page actions on one row** (owner QA; DESIGN.md §3 "Pencarian dan aksi halaman satu baris").

- **Layout.**
  - The "Dashboards" title sits alone on its row.
  - Under it, one row: the search takes the remaining width, then the Table / Gallery switch, the page ⋯ and **+ New**, all 44 px tall and vertically centred.
  - The view switch moved there from the filters row, because the rule names it among the page actions.
- **Phone: one row, the cleaner of the two options.**
  - The row holds the search, the ⋯ and an icon-only New. "New" stays as screen-reader text.
  - The view switch stays in the filters row, so the search keeps its room.
- **Measured in Chrome over CDP, light theme, Table view:**

  | Viewport | Title (top) | Search, ⋯ and New (top / height) | View switch (top / height) |
  | --- | --- | --- | --- |
  | 1366x900 | 88 | 132 / 44 | 132 / 44 |
  | 390x844 | 80 | 124 / 44 | not on this row |

- **Tests.** happy-dom has no layout, so the unit test checks the DOM: the search, New, ⋯ and the view switch share one row element, and the title is above it, alone. The 1-px `top` check comes from the browser measurements above.

**SHOULD-FIX 2: the frame follows the shell's reserve.**

- `PageFrame` (`library-parts.tsx`) now reads `--content-bottom`, the token BI-58 adds (`c4d87211`, `50295df6`).
- Its fallbacks are today's values:
  - **With the dock:** a 5rem reserve, plus the negative margin that gives back `pb-24` / `lg:pb-28`.
  - **Without the dock:** the shell's own bottom padding.
- Heights are unchanged on this branch. Once BI-58 merges, the frame ends where `--content-bottom` says.
- With the token present, the margin works out at `--content-bottom - 6rem`. That only shortens `main` below the frame and moves nothing on screen. It can go once this branch has the token.
- No BI-58 code was copied.

**Checks** (each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`, one at a time):

- `bun test` on three files (`dashboard-list-page.test.tsx`, `template-pages.test.tsx`, `library-item-menu.test.tsx`): 37 pass, 0 fail.
- `bun --bun eslint src/features/dashboards/`: clean.
- `bun run typecheck`: clean.
- **Not verified:** the full `bun run lint` and `bun run test`. Both are left to CI.

**Screenshots.** The 12 Table and Gallery shots in `docs/plans/bi-59-screens/` were retaken with this header. A brief dev server on 28622 and one headless Chrome were stopped by PID afterwards. 28620 was not touched.

- No horizontal scroll at any size.
- The five `state-*.png` shots predate this round and still show the old header.

### Handoff, BI-59 review round 2 (developer, 2026-11-14)

**SHOULD-FIX 3: one compact header row** (owner QA, "tingginya gede banget"; DESIGN.md §3 "Judul, pencarian, dan aksi halaman satu baris yang ringkas").

- **Layout.** One row holds the title, the search (it takes the remaining width), the Table / Gallery switch, ⋯ and + New.
  - Every control is 36 px tall, the console's `h-9`, and the title is centred on them (`leading-9`).
  - The list's top margin went from `mt-5` to `mt-4`.
  - The type filter and the folder picker stay on the list's own row.
- **Shared look.** The controls are the shadcn components, sized to our tokens, with `rounded-lg` and no border:
  - the search, the switch and ⋯ share one soft fill (`bg-muted/70`, `dark:bg-input/30`);
  - ⋯ is the ghost `Button` at `icon-lg` with that fill, not the default outline button;
  - New is the primary `Button` at `lg` (h-9).
- **Phone.** The title has its own row. Under it: the search, ⋯ and an icon-only New. The view switch stays in the filters row.
- **Measured in Chrome over CDP:**

  | Viewport | Title | Search | Switch | ⋯ | New | Header height | Recent / list starts |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | 1366x900, all four Table/Gallery and light/dark shots | 88 / 36 | 88 / 36 | 88 / 36 | 88 / 36 | 88 / 36 | 36 (was 44 + 16 + 28) | 140 |
  | 390x844 | 80 / 36 | 128 / 36 | in the filters row | 128 / 36 | 128 / 36 | n/a | n/a |

  Each cell is top / height in px. No horizontal scroll at either size.
- **Test.** The header is one flex row (no wrap at desktop width). The title, the search, the switch, ⋯ and New are five separate direct children of it.
- **Checks** (each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`, one at a time):
  - `bun test` on three files (`dashboard-list-page.test.tsx`, `template-pages.test.tsx`, `library-item-menu.test.tsx`): 37 pass, 0 fail;
  - eslint on `src/features/dashboards/`: clean;
  - `bun run typecheck`: clean.
  - **Not verified:** the full `bun run lint` and `bun run test`, left to CI.
- **Screenshots.** Retaken at 1366: `{table,gallery}-{light,dark}-1366x900.png`.
  - The 1190 and 390 shots, and the `state-*` shots, show the earlier headers.
  - A brief dev server on 28622 and one headless Chrome were stopped by PID afterwards. 28620 was not touched.

### Handoff, BI-59 review round 3 (developer, 2026-11-14)

SHOULD-FIX 4 (owner QA; DESIGN.md §3, a search of normal width):
- **Layout.** The header row is now the title, then an empty gap that takes the spare width, then a 288 px search (`w-72`), the switch, ⋯ and New, all 36 px. The search widens to 360 px while focused, over 200 ms, with no animation under reduced motion. The phone layout is unchanged.
- **Measured** in Chrome at 1366x900, dark: the search is 288 px wide at top 88 px, the gap is 434 px, and nothing scrolls sideways. `table-dark-1366x900.png` is retaken.
- **Checks**, each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`:
  - `bun test` on three files (`dashboard-list-page.test.tsx`, `template-pages.test.tsx`, `library-item-menu.test.tsx`): 37 pass, 0 fail. The structure test now also asserts that the gap is the flexible child and the search is not.
  - eslint on `src/features/dashboards/`: clean.
  - `bun run typecheck`: clean.
