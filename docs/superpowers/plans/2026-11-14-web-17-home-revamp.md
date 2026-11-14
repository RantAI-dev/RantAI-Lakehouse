# WEB-17: Home revamp (mockup B), plan

| | |
| --- | --- |
| Backlog | `WEB-17` |
| Spec | `docs/core/specs/web-17.md` |
| Feature page | `docs/core/features/console-home.md` |
| Base | `feat/bi-61-dashboard-page` at `8d401544` (shell: 48 px navbar, `--app-navbar-h`, `Bot` mark, one loading look). This PR follows BI-61 |
| Branch | `feat/web-17-home-revamp`, worktree `/home/hv/mtharreq/lakehouse-web17` |
| Planner and reviewer | Claude Opus (hub session) |
| Developer | an Opus developer agent (console design work, AGENTS.md exception) |

## 1. Decisions already made

- **Layout is mockup B** (owner, 2026-11-14): `design/chart-builder-mockups` at `a7779918`, `/mockups/home/b`, code in `src/features/mockups/home-history/` (`home-mockups.tsx`, `widgets.tsx`, `lake-art.tsx`, `shared.tsx`, `home-history.css`) in the worktree `/home/hv/mtharreq/lakehouse-mockups`. Port from it; it is our own code. The owner looked at the **Dither** style in dark mode and liked it.
- **Background choice** (owner, 2026-11-14): Home's Customize menu gets **Background: Dither (default) · ASCII · Off**, remembered in `localStorage` (same approach as the Dashboards table/gallery choice). Off = plain page, the bento moves up, no empty band. DESIGN.md: "Latar dekoratif di aplikasi bisa dimatikan pengguna".
- **Only real data** (spec "Not in this item"): no activity heatmap, no month totals, no "Gold rows this month", no "opened N h ago". Use these instead:
  - **Focal dashboard:** the existing preview choice (`shown.previewBoardId`, else last board, else most recently updated with charts; `home-page.tsx:341-358`) drawn live with `DashboardPreview` (`home-page.tsx:459`). If the person has favourites (`dashboardService.shortcuts()`), prefer the first favourite when no preview board was chosen.
  - **Recent AI chats:** `copilot.sessions` (`home-page.tsx:371-377`), latest 4, "N more" → `/copilot/history`. Time is "updated", the only time the API has.
  - **Pipelines:** `pipelineService.listPipelines` (pipeline level: `status`, `lastRunAt`, `nextRunAt`; `home-pipelines.ts:35-52`). A failed pipeline offers **Ask why**: fills the composer with a question naming the pipeline, focus in the composer, nothing sent.
  - **Needs attention:** open alerts (`overviewService.listAlerts`), unhealthy/degraded sources (`connectorService.listConnectors`), stale assets (`overviewService.getSummary().staleAssets`). This replaces the floating "Status is incomplete · Health" pill; when all checks pass it reads as one calm line ("All clear" with the time), not an empty card. A check that could not be read says so honestly (keep `homeStatus()` semantics, `home-status.ts`).
  - **Composer context strip:** "Reads the N tables you can see · Answers only, changes nothing" for Ask (count = `gold.listMarts().length`; if the read fails, drop the count, keep the sentence); Build mode shows what Build may change, in one line.
- **Keep working behaviour:** the composer's Ask/Build handoff to `/copilot` (`home-page.tsx:196-208`), `homePrompts()` chips, the server-side Home layout (`use-home-layout.ts`, `/api/home/layout`) for which cards and shortcuts show; the Customize dialog keeps its current options and gains Background.
- **Brand and shell:** the frame is the real BI-61 shell (not the mockup's static frame). Product name "RantAI Lakehouse". No sparkles icons; AI = `Bot`. Greeting uses the signed-in user's name (`useAuth().user`).
- **Loading:** one loading look that matches the page (DESIGN.md "satu halaman, satu tampilan loading"); the scene can appear as its solid base colour first, then the drawing fades in once.
- **Motion:** the scene draws once after first paint (it must never block the composer or the LCP). Optional slow shimmer/ripple only if cheap (≤10 fps redraw or CSS transform/opacity), paused off-screen and in a hidden tab; still under `prefers-reduced-motion`. Reuse `use-in-view`-style gating; check if a shared hook exists first (rule 4).
- **Skills:** load `emil-design-eng` and `shadcn` before building. DESIGN.md wins over any skill.
- **Out of scope:** the History / AI chats page (on hold, owner has a new idea), any Rust/API change, the landing site.

## 2. Anchors (at `8d401544`)

- `src/features/home/home-page.tsx` (867 lines): `HomePage` `:166`, `ask()` `:199-208`, statuses `:262-339`, preview board choice `:341-358`, recent `:360-389`, prompts `:393-398`, `DashboardPreview` `:459`, composer `:542`, `StatusLine` `:720`, `AiButton` `:832`, `greeting()` `:862`.
- `src/features/home/home-customize.tsx` (Customize dialog), `use-home-layout.ts`, `home-ui.tsx`, the cards `open-alerts-card.tsx`, `pipeline-runs-card.tsx`, `recent-card.tsx`, `saved-queries-card.tsx`, `sources-card.tsx`, `start-button.tsx`, `home-shortcuts.ts`.
- `src/lib/home-status.ts`, `home-prompts.ts`, `home-pipelines.ts`, `home-recent.ts`.
- Services: `gold.listMarts()` (`src/services/clients/gold.ts:64`), `dashboardService.shortcuts()` (`dashboards.ts:553`), `chatSessionService` via `use-copilot.ts:316-324`.
- Tests: `src/features/home/home-page.test.tsx` and any `src/lib/home-*.test.ts`.

## 3. Tasks (one commit each, in order)

1. **T1, before screenshots** of `/` at 1366×800 and 1366×900, light and dark, plus 390×844, into `docs/plans/web-17-screens/before-*`.
2. **T2, the scene:** port `lake-art.tsx` as a production component (`src/features/home/lake-scene.tsx` or similar): Dither and ASCII styles, light (navy and lake-blue ink on cream) and dark (lake blue and cream on deep navy), fades into the page background. Drawn once after first paint; cheap optional motion with gating; static under reduced motion; sized to the hero band only (no full-page canvas). Unit test for the style/off switch logic.
3. **T3, layout:** greeting + composer + context strip + chips over the scene; bento below (focal dashboard, Recent AI chats, Pipelines, Needs attention), all within 1366×800 above the fold for the composer, chips and the focal dashboard's first tile row. Remove the floating status pill and the sparkles buttons (status moves into Needs attention; the create shortcuts stay reachable via Customize's shortcuts, shown compactly if enabled). Phone: single column, no horizontal scroll, scene shorter.
4. **T4, Background choice** in Customize (Dither / ASCII / Off), remembered in `localStorage` with try/catch; Off closes the band up. Test that the choice persists and Off renders no scene.
5. **T5, states:** loading (one look, matches the page), empty (new user: no dashboards, no chats, no pipelines; each widget has a designed empty state that offers the next step), error (a widget whose read failed says so in one line, others still render).
6. **T6, Ask why** on failed pipelines; composer prefill; test.
7. **T7, after screenshots** (same sizes, light/dark, Background Off, empty state) into `docs/plans/web-17-screens/after-*`; check each yourself.
8. **T8, handoff** in §6.

## 4. Machine rules (owner)

- No Rust. Never the full `bun run test` or `bun run lint`. Run `bun run typecheck`, `bun test src/features/home src/lib/home-*.test.ts` (and any test you touch), `bun x eslint <changed files>`.
- Before each heavy command `free -m` must show ≥3 GB available; run as `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15 <cmd>`, one at a time.
- **Only one Next dev server on the VM.** The owner's mockup server runs on 28620 as a production server (`next start`, not a dev server), leave it alone. Use port **28699** for your dev server if free, plus one headless Chrome on a free port in 28690–28696; stop both by PID (never `pkill -f`/`pgrep -f`). `next dev` rewrites `AGENTS.md`: `git checkout AGENTS.md`, never commit it.
- For real data, point the dev server's API proxy at the QA API on 28604 (read how `.env`/next config proxies `/api`); do not restart or change anything on 28604/28605. Login: `owner@example.invalid`, password from `/home/hv/mtharreq/lakehouse-main/.env` line 58 (`AUTH_BOOTSTRAP_PASSWORD`), read into a variable, never printed.
- Never touch ports 3000, 3200, 3450, 28604, 28605, 28620. No prettier.
- Public AGPL repo: the scene is our own code; nothing from React Bits, the DCCM reference or any traced image.

## 5. Review

(planner)

## 6. Handoff

(developer)
