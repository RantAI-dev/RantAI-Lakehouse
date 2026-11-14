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

## 4. Machine rules (owner)

- No Rust. Never the full `bun run test` or `bun run lint`. Run `bun run typecheck`, the touched test files (`bun test src/components/app-shell src/features/copilot` and others you change), `bun x eslint <changed files>`.
- Before each heavy command `free -m` must show ≥3 GB available; run as `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15 <cmd>`, one at a time.
- **Only one Next dev server on the VM.** Another developer (WEB-17) may be using port 28699. Write the code first; start your dev server on 28699 only when no `next dev` is running (check `ss -ltnp` and `ps`), otherwise wait for it to stop; never kill it. Headless Chrome on a free port in 28690–28696. Stop yours by PID (never `pkill -f`/`pgrep -f`). `next dev` rewrites `AGENTS.md`: `git checkout AGENTS.md`, never commit it. 28620 is a production mockup server; leave it.
- For real data, point the dev server's API proxy at the QA API on 28604 (read how `.env`/next config proxies `/api`); do not restart or change anything on 28604/28605. Login: `owner@example.invalid`, password from `/home/hv/mtharreq/lakehouse-main/.env` line 58 (`AUTH_BOOTSTRAP_PASSWORD`), read into a variable, never printed. The QA user may have few or no conversations: create two or three through the UI (Home composer, Ask) so the list has rows.
- Never touch ports 3000, 3200, 3450, 28604, 28605, 28620. No prettier.

## 5. Review

(planner)

## 6. Handoff

(developer)
