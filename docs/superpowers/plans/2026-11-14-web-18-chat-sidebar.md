# WEB-18: AI chat history lives in the sidebar, plan

| | |
| --- | --- |
| Backlog | `WEB-18` |
| Base | `feat/bi-61-dashboard-page` at `8d401544` (shell: 48 px navbar, `--app-navbar-h`, sidebar collapse motion, `Bot` mark) |
| Branch | `feat/web-18-chat-sidebar`, worktree `/home/hv/mtharreq/lakehouse-web18` |
| Related | `WEB-17` (Home revamp, `feat/web-17-home-revamp`) links Home's "Recent AI chats" to `/copilot?session=<id>`; it does not touch the sidebar |
| Planner and reviewer | Claude Opus (hub session) |
| Developer | an Opus developer agent (console design work, AGENTS.md exception) |

## 1. Decisions already made (owner, 2026-11-14)

Owner: "halaman menu history itu dihapus aja, dan cukup ditampilkan dari menu Home. Pas sudah di sesi conversation, sidebar itulah yang jadi list historynya, jadi kek Claude. Di atas ada button untuk back ke menu sidebar yang asli. Tambahin animasi geser sidebarnya biar lebih keliatan." Reference: a chat app sidebar with "‹ Back to …", "+ New chat", "Search", "AI settings", then the list of chats.

- **No History page and no History sidebar entry.** Remove the `History` group from `nav-config.ts:92-103`, the `HIGHLIGHT_UNDER` rule (`:224-230`) and the route guard entry (`:282`). `/copilot/history` keeps working as a redirect: to the most recent conversation (`/copilot?session=<id>`), or to Home when there is none. Delete `copilot-history-page.tsx` once nothing imports it (keep any logic worth keeping, e.g. grouping by day and search, by moving it into the new list).
- **On a conversation (`/copilot?session=…`), the app sidebar shows the chat list instead of the main menu.** Top to bottom:
  1. **"‹ Back to menu"**: slides the main menu back in. It does not leave the conversation.
  2. **"+ New chat"**: goes to Home with the composer focused (the product rule stays: a conversation starts on Home, `copilot-page.tsx:62-74`).
  3. **"Search"**: filters the list in place (title and preview, as the History page did); "/" focuses it while the chat list is shown; Esc clears.
  4. **"AI settings"**: the existing assistant settings page/section (find its route; do not invent one).
  5. **The chats**, newest first, grouped Today / Yesterday / Previous 7 days / Earlier, from `copilot.sessions` (`use-copilot.ts`), with infinite paging if the list is longer than one page. Each row: title only, one line, truncated; the open one highlighted like an active nav item; hover/focus shows ⋯ with Rename and Delete (reuse the existing session actions in `session-parts.tsx` / `history-menu.tsx`; Delete has an undo toast as History had, or a confirm if that is what exists). Mode (Ask/Build) as a small icon only if it does not crowd the row.
- **When the main menu is shown on a conversation page** (after "Back to menu"), the sidebar has a way back to the chat list: the main menu's AI entry point. Add one main-menu row **"AI chats"** (`Bot` icon) that is shown only while on `/copilot` and slides the chat list back in; elsewhere it is not shown. (Off `/copilot`, chats are reached from Home and from the navbar's AI Copilot panel.)
- **Leaving the conversation** (clicking any main-menu link, the logo, or browser back to a non-copilot page) shows the main menu as usual.
- **Motion:** the two views slide horizontally inside the sidebar, the main menu out to the left and the chat list in from the right (and the reverse for Back), about 220–260 ms, strong ease-out (`cubic-bezier(0.23,1,0.32,1)`, the S2 curve), both views moving together so there is never an empty sidebar frame; the sidebar's width does not change; reduced motion = instant swap. Load `emil-design-eng` and follow DESIGN.md over it.
- **Collapsed (icon) sidebar on a conversation:** show icon buttons for Back to menu, New chat, Search and AI settings, and no chat rows (a tooltip on each). Opening the sidebar shows the full list.
- **Phone:** the sidebar sheet shows the chat list on `/copilot`, with the same Back to menu.
- **The conversation page's own history affordances go away** so there is one place for chat history (DESIGN.md "satu aksi, satu tombol", "satu panel samping"): the History button/`ConversationPanel` on `/copilot` (`copilot-page.tsx`, `conversation-panel.tsx`), and links to `/copilot/history` in `session-parts.tsx:95`, `history-menu.tsx:58`, `recent-card.tsx:83` (Home's card: point it to `/copilot?session=<latest>`; `WEB-17` replaces that card anyway). The navbar's AI Copilot side panel (`copilot-sidebar.tsx`, `copilot-dock.tsx`) keeps whatever session switching it has today.
- **Skeleton:** `AppShellSkeleton` on `/copilot` shows the chat-list shape (BI-61's one-loading-look rule).
- **Out of scope:** Home (WEB-17), the AI Copilot side panel's own design, any Rust/API change.

## 2. Anchors (at `8d401544`)

- `src/components/app-shell/nav-config.ts` (History group `:92-103`, `HIGHLIGHT_UNDER` `:224-230`, guard `:282`), `app-sidebar.tsx` (405 lines; `SidebarBrand`, `SIDEBAR_*` classes, collapse motion from S2), `app-shell-skeleton.tsx`, `shell-placeholder.ts`.
- `src/app/copilot/page.tsx`, `src/app/copilot/history/page.tsx`; `src/features/copilot/copilot-page.tsx` (202 lines; `SessionUrlSync` and `?id=`/`?session=` at `:28-31`, the title bar and History at `:113-136`, `:194`), `conversation-panel.tsx`, `copilot-history-page.tsx`, `history-menu.tsx`, `session-parts.tsx`, `use-copilot.ts`.
- Tests: `src/components/app-shell/*.test.*`, `src/features/copilot/copilot-page.test.tsx`, any test that names `/copilot/history`.

## 3. Tasks (one commit each, in order)

1. **T1, before screenshots** of `/copilot?session=<id>` and `/copilot/history`, 1366×900 light and dark, into `docs/plans/web-18-screens/before-*`.
2. **T2, chat-list sidebar view** with Back to menu, New chat, Search, AI settings and the grouped list (rename/delete/paging), shown on `/copilot`; "AI chats" row in the main menu only on `/copilot`. Tests: the view switches by route; Back to menu keeps the page; search filters.
3. **T3, motion** (slide between the two views, reduced motion instant), plus collapsed icon mode and the phone sheet.
4. **T4, remove the History page and entry:** nav-config, redirect `/copilot/history`, delete the page and the conversation page's History panel, repoint every link. Tests updated.
5. **T5, skeleton** on `/copilot` matches the chat list.
6. **T6, after screenshots** (same sizes; plus a 0/80/160/240 ms frame sequence of the slide) into `docs/plans/web-18-screens/after-*`.
7. **T7, handoff** in §6.

### Slice 2 (owner QA, 2026-11-14)

8. **T8, Home becomes a split row; the "AI chats" row goes.** Owner: "di menu Home ini tu, buat button split juga untuk ke history copilot". Home gets the same split as the Dashboards row (`action` in `nav-config.ts`, drawn by the split branch of `renderEntry` in `app-sidebar.tsx`): the label part goes to `/`, the square right part (icon `MessagesSquare`, label and delayed tooltip "AI chats") **slides the sidebar to the chat list in place, on any page, without navigating**. Opening a chat from the list navigates to the conversation as today. Back to menu slides back. The view state is no longer only route-driven: on `/copilot` the list is the default as today; elsewhere the menu is the default; the right part opens the list; any navigation resets to that route's default. The right part is lit (`aria-current`) on `/copilot`, and the Home label part is then not lit. Remove `aiChatsEntry`. Collapsed icon mode hides the right part, as with Dashboards. The skeleton draws the Home row split too. Tests: the right part opens the list on `/` and on another page without a route change; Back to menu returns; no "AI chats" row anywhere; the focus lands on Back to menu after the slide in and on the right part after the slide out.

## 4. Machine rules (owner)

- No Rust. Never the full `bun run test` or `bun run lint`. Run `bun run typecheck`, the touched test files (`bun test src/components/app-shell src/features/copilot` and others you change), `bun x eslint <changed files>`.
- Before each heavy command `free -m` must show ≥3 GB available; run as `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15 <cmd>`, one at a time.
- **Only one Next dev server on the VM.** Another developer (WEB-17) may be using port 28699. Write the code first; start your dev server on 28699 only when no `next dev` is running (check `ss -ltnp` and `ps`), otherwise wait for it to stop; never kill it. Headless Chrome on a free port in 28690–28696. Stop yours by PID (never `pkill -f`/`pgrep -f`). `next dev` rewrites `AGENTS.md`: `git checkout AGENTS.md`, never commit it. 28620 is a production mockup server; leave it.
- For real data, point the dev server's API proxy at the QA API on 28604 (read how `.env`/next config proxies `/api`); do not restart or change anything on 28604/28605. Login: `owner@example.invalid`, password from `/home/hv/mtharreq/lakehouse-main/.env` line 58 (`AUTH_BOOTSTRAP_PASSWORD`), read into a variable, never printed. The QA user may have few or no conversations: create two or three through the UI (Home composer, Ask) so the list has rows.
- Never touch ports 3000, 3200, 3450, 28604, 28605, 28620. No prettier.

## 5. Review

(planner)

## 6. Handoff

### Slice 1 (T1–T6), developer (Opus, console design work), 2026-11-14

**Commits** (on `feat/web-18-chat-sidebar`, not pushed):

| Task | Commit | What |
| --- | --- | --- |
| T1 | `268477d7` | Before screenshots, `/copilot?id=` and `/copilot/history`, 1366×900 light and dark |
| T2 | `1ef8bf85` | `SidebarChats` (Back to menu, New chat, Search, AI settings, chats by day, paging, ⋯ Rename/Delete), "AI chats" row on `/copilot` only, `use-chat-list.ts`, `sidebar-classes.ts` |
| T3 | `106ee83f` | The slide between the two views, focus follows, collapsed icons, phone sheet |
| T4 | `f03d8eae` | History entry, page, panel and switcher removed; `/copilot/history` → `/copilot?open=latest`; links repointed |
| T5 | `43a65ac4` | `AppShellSkeleton` draws the chat list on `/copilot` (`SidebarChatsSkeleton`, shared `ChatListSkeleton`) |
| T6 | `89a64333` | After screenshots and the slide's 0/80/160/240 ms frames, both ways |

**How it works.**
- `AppSidebar` keeps both views mounted side by side in `SidebarContent` (`relative overflow-hidden`), each `absolute inset-0` (`SIDEBAR_VIEW_PANE_CLASS`). On `/copilot` (with an assistant permission) the chats are shown; "Back to menu" sets `menuOnConversation`, which any pathname change resets, so it never leaves the page. The menu then has an active "AI chats" row (`Bot`, chevron) right under Home, only on `/copilot`, which slides the chats back.
- Slide: `transform` only, 240 ms, `cubic-bezier(0.23,1,0.32,1)` (the sidebar's curve); the menu goes to `-translate-x-full` while the chats come from `translate-x-full`, both at once, so the frame is never empty and the width never changes. The out-of-view pane is `inert`. Reduced motion: `motion-reduce:transition-none`, an instant swap. The chat pane mounts off-screen the first time it is wanted (forced layout, then flipped) so arriving from Home also slides; it stays mounted after. Focus moves to the control that leads back (AI chats / Back to menu).
- Collapsed: the four actions are the same rows, narrowed to their icons with tooltips (`SidebarMenuButton tooltip`); Search's row becomes a button that opens the sidebar and focuses the field; the chat list fades and is `inert`.
- Phone: the sheet holds the same `AppSidebar`, so it opens on the chat list on `/copilot`, with Back to menu; picking a chat, New chat or AI settings closes the sheet.
- Search: server-side `q` (title and message text, as History did), debounced 300 ms; "/" focuses it while the list is shown unless typing in a field; Esc clears, a second Esc blurs.
- The list: `useChatList` (the History page's infinite query, 30 a page, `IntersectionObserver` inside the list's own scroller, `.scroll-fade` edges); `useFollowRecentSessions` invalidates it whenever the Copilot's recent list changes (a chat saved, renamed, deleted or answered). Rows: title only, one line, active highlighted like a nav item, ⋯ on hover/focus (always shown on touch) with the existing `SessionActionsMenu` (Rename dialog; Delete is the existing confirm dialog, there was no undo toast).
- AI settings is `/admin/settings?section=assistant` (`ADM-9`'s Assistant section), shown only with `settings:write`, which that section requires.

**Links repointed, and `/copilot/history`.**
- `nav-config.ts`: History group, `HIGHLIGHT_UNDER`, the `/copilot/history` `ROUTE_ACCESS` entry removed; `/copilot` highlights no menu row.
- `/copilot/history` is an HTTP 307 in `next.config.ts` `redirects()` to `/copilot?open=latest`; `SessionUrlSync` then lists one conversation and replaces the address with `/copilot?id=<newest>`, or `/` when there is none or the list fails (verified in the browser: it landed on `/copilot?id=c_d3b4a531`). The page file is deleted. **Plan mismatch:** the plan said "keeps working as a redirect" with the guard entry removed; a page calling `redirect()` does not work here, because the redirect is only thrown when the client renders the page and `RouteGuard` renders "You don't have access" for an address without a `ROUTE_ACCESS` entry. **Pre-existing bug, not fixed:** `/admin/maps` (checked in the browser, it stays on "You don't have access") and the other legacy redirect pages have the same problem.
- `session-parts.tsx` (AI Copilot panel's history menu): "View all history" → "View all chats", `/copilot?id=<open or newest>`, hidden with no chats.
- `history-menu.tsx`: `CopilotHistoryMenu` (with its History link) deleted; `CopilotHistoryButton` kept for the dock and side panel.
- `recent-card.tsx` (Home): "History" → "Chats", the newest conversation the card lists, hidden with none.
- `conversation-panel.tsx` (its "All history" link) and `copilot-history-page.tsx` deleted.
- `docs/OPERATIONS.md` role notes updated. **Not touched:** `docs/core/features/copilot-chat.md` open question 2 still says the entry is "History" (planner's page).

**Judgment calls to review.**
1. The conversation page's own **New chat** button was removed too, not only History ("satu aksi, satu tombol": the sidebar's New chat is beside it). The bar holds the title alone, 36 px high as before, so the chat did not move.
2. Groups use the existing `groupSessions` (Today / Yesterday / Previous 7 days / **Previous 30 days** / Earlier), one more bucket than the plan lists, so History and the sidebar agree.
3. No mode (Ask/Build) icon on rows: the row is title only.
4. The class constants moved from `app-sidebar.tsx` to `sidebar-classes.ts` (no import cycle with `sidebar-chats.tsx`).

**Commands** (on the final commit `89a64333`'s code; each heavy one under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`):
- `bun run typecheck`: exit 0.
- `bun test src/components/app-shell src/features/copilot src/features/home`: 215 pass, 0 fail, 24 files.
- `bun x eslint <the 19 added or modified .ts/.tsx files>`: 0 problems.
- Per commit: `bun test src/components/app-shell src/features/copilot` (T2 201/0, T3 204/0, T4 212/0 with `src/features/home`, T5 213/0), typecheck and eslint on the touched files, all clean.
- Not run (machine rules): full `bun run test`, `bun run lint`, any Rust.

**Screenshots:** `docs/plans/web-18-screens/before-*` (4) and `after-*` (19, including `after-slide-out-{000,080,160,240}ms` and `after-slide-in-…`), taken against the QA API on 28604 through a dev server on 28699 and headless Chrome on 28691, both stopped by PID afterwards. The QA model answers 401, so a question asked on Home is never saved; the four QA conversations were saved through `POST /api/ai/sessions` as the QA user, with an assistant line saying so. All are "Today", so the other day groups were not seen in the browser (unit-tested through `groupSessions`). `after-row-hover-light` and `after-search-light` predate the title bar's 40→36 px correction.

**Not verified:** a real phone (iPhone Safari), only Chrome's mobile emulation and happy-dom; infinite paging past 30 chats in the browser (QA has 4); Rename/Delete from the sidebar in the browser (unit paths are the existing ones); keyboard-only walkthrough in the browser beyond the "/" and focus tests; dark-mode shots of hover, search, collapsed and the slide.

### Slice 2 (T8), developer, 2026-11-14

**Commit:** `11c7772b` `feat(copilot): WEB-18 T8, Home split row opens the chat list` (not pushed).

- `nav-config.ts`: `NavItem.action` is now `{title, icon}` plus either `href` (link, Dashboards) or `opens: "chats"` (Home, `MessagesSquare`, "AI chats"). Home carries it, so it is shown only with `assistant:use` or `assistant:ask`.
- `app-sidebar.tsx`: one split branch for both rows; the chats action renders a `<button data-view-focus="menu">` in the same tooltip provider, 32 px square, separator and colours. `menuOnConversation` became `chatsChoice` (`boolean | null`, reset to `null` on any pathname change); `showChats = hasAssistant && (chatsChoice ?? onConversation)`. Lit (`aria-current="page"`) on `/copilot`, so the Home label part is not. `aiChatsEntry`, its `menuEntries` case and the `Bot` import are gone. The slide, mount-then-move and focus handoff are unchanged.
- `app-shell-skeleton.tsx`: no change needed, it already draws a split row for any item with `action`, so Home is drawn split; its test does not count rows.
- `sidebar-chats.test.tsx`: right part opens the list on `/`, `/dashboards` and returns, with no `router.push/replace`; the one "AI chats" button is Home's part (no row); lit on `/copilot`; hidden without the assistant; focus on Back to menu after sliding in and on the part after sliding out. `app-sidebar.test.tsx` unchanged (Dashboards split tests pass).

**Commands:** `bun run typecheck`: no errors. `bun test src/components/app-shell`: 72 pass, 0 fail, 10 files. `bun x eslint nav-config.ts app-sidebar.tsx sidebar-chats.test.tsx`: 0 problems. Each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`.

**Not verified:** in a browser (no dev server run), the tooltip and lit state visually, the phone sheet, dark mode, `src/features/copilot`/`home` tests (not run), and the Home label's "not lit" check is weak (`data-active` is not asserted positively elsewhere).
