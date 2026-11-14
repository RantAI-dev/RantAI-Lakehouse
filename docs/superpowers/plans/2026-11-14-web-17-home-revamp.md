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
- **Background choice** (owner, 2026-11-14): Home's Customize menu gets **Background: Dither (default) · ASCII · Off**, remembered in `localStorage` (same approach as the Dashboards table/gallery choice). ~~Off = plain page, the bento moves up, no empty band.~~ Changed 2026-11-14 (owner QA): **Off keeps the layout; only the art is hidden**: the band keeps its height, the greeting, composer and chips stay where they are, the band shows the plain page, and switching cross-fades the art in place. Owner: "kalo pas off jangan naik ke atas gini composernya, tetap aja kek gitu tapi kosong aja backgroundnya". DESIGN.md: "Latar dekoratif di aplikasi bisa dimatikan pengguna".
- **Greeting over the art without a hole** (owner QA, 2026-11-14): the art stays whole behind "Good morning, …" ("aku gasuka karena ini kek gambarnya hilang biar tulisan Good morning-nya tetap keliatan… biar tetap full backgroundnya muncul tapi tulisan itu tetap keliatan"). The text is readable through a soft veil over its own box (dots thin a little and lean towards the band's base colour, smooth falloff, no edge) and a soft halo in the base colour on the text. No hard cut-outs around the composer either.
- **Day and night follow the theme** (owner, 2026-11-14: "kalo buat light mode berarti kondisinya dibuat siang juga, matahari muncul di sebelah kiri; kalo dark berarti matahari turun dan bulan naik di sebelah kanan"). Light theme: a day lake (lighter sky in the same dither/ASCII language, the sun on the left, sparkling water, the whale at the waterline). Dark theme: night (the moon on the right, stars). A theme switch on Home animates on the canvas: the moon sets on the right while the sun rises on the left (or the sun sets on the left below the waterline, its reflection fading, while the moon rises on the right), sky and water tones blend, 1.5 s eased, up to 30 fps only during the change, then back to the idle loop (≤10 fps, paused off screen and in a hidden tab). Reduced motion: the end state at once. The base colours (cream by day, navy by night) are the placeholder's, so the first paint does not flash.
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
4. **T4, Background choice** in Customize (Dither / ASCII / Off), remembered in `localStorage` with try/catch; Off hides the art and keeps the layout (owner QA 2026-11-14, see §1). Test that the choice persists, Off hides the art and the hero does not move.
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

### Slice 1 (the whole PR), 2026-11-14, Opus developer agent

Branch `feat/web-17-home-revamp`, base `50451d5a`. Not pushed.

| Task | Commit |
| --- | --- |
| T1 before screenshots | `5d555fb4` |
| T2 the scene | `558bfdf8` |
| T3 layout | `deed7120` |
| T4 Background choice | `e4380100` |
| T5 states + Home loading look (planner's mid-task addition) | `dcbc0867` |
| T6 Ask why | `1ba02d85` (test only; the behaviour landed with the Pipelines widget in T3) |
| fixes found while checking T7 | `11e9d399` (focal waits for favourites; orchestrator-unreachable wording), `a87361c7` (phone greeting patch) |
| T7 after screenshots | `bea11bf3` |
| T8 this entry | the commit that adds it |

**The scene** (`src/features/home/lake-scene.tsx`): one 2D canvas sized to the hero band (300 px phone, 400 px from `sm`), never the page. The first drawing waits for `requestIdleCallback` after first paint (fallback `setTimeout`), then fades in once (500 ms, skipped under reduced motion); until then the band is its solid base colour. The still sky (moon, stars, whale above water) is computed once per size and theme; only the water rows are recomputed, at most 10 fps (`FRAME_MS = 100`), only while an `IntersectionObserver` says it is on screen and `document.hidden` is false. Dither is one pixel per 3 px block scaled up without smoothing; ASCII draws glyphs per colour over a cached sky layer. Under `prefers-reduced-motion` it draws once and stops. A `ResizeObserver` redraws only on a real box change.

**Cost**, headless Chrome 148 on this VM (software rendering, `--disable-gpu`), 1366×800, dark, `Performance.getMetrics` `TaskDuration` over 10 s, canvas hashed before/after:

| Case | Main thread busy | Script | Canvas changed |
| --- | --- | --- | --- |
| Dither, visible | 10.4 % | 6.1 % | yes |
| ASCII, visible | 12.5 % | 5.1 % | yes |
| Dither, reduced motion | 1.4 % | 0 % | no (still) |
| Dither, scrolled off screen (1366×450, band bottom at −66 px) | 1.7 % | 0.2 % | no (paused) |
| Background Off (baseline) | 1.1 % | 0 % | no |

**Data per widget** (all existing APIs, nothing sample):
- Focal dashboard: `dashboardService.listBoards` + `dashboardService.shortcuts()` favourites + `readLastBoard()`; order in `lib/home-focal` (chosen in Customize → first favourite → last opened → most recently updated; charts only). Tiles from `/api/dashboard?board=` via `DashboardPreview` (now tiles only, no box per tile). Meta line: why it is shown + `updatedAt`.
- Recent AI chats: `copilot.sessions` (latest 4, `updatedAt`); `use-copilot` now reports `sessionsRead` (loading/ok/error). Rows open `/copilot?id=<id>` (the param Home already used; `/copilot` reads `id` or `session`). "All chats" opens the latest conversation, hidden with no chats (planner's correction).
- Pipelines: `pipelineService.listPipelines` → `pipelineRows(…, 4)`; status, last run, next run; failed rows offer Ask why. A 503 with `error` and no rows reads "Couldn't reach the orchestrator to list pipelines."
- Needs attention: `homeStatus()` unchanged over pipelines, `overviewService.listAlerts`, `connectorService.listConnectors`, `overviewService.getSummary().staleAssets`; rows list alerts, sources, datasets (failing pipelines are counted in the line but listed only in Pipelines). Clear = one line "All clear · checked HH:MM".
- Context strip: `goldService.listMarts().length` ("Reads the N tables you can see"); count dropped if the read fails.

**Commands run** (each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`, one at a time, after `free -m` showed ≥ 3 GB available), final run on `bea11bf3` plus this entry:
- `bun run typecheck`: exit 0.
- `bun x eslint <the 24 changed .ts/.tsx files under src>`: exit 0, 0 errors, 1 warning (`changedABoard` unused in `use-copilot.ts:9`, present at the base).
- `bun test src/features/home src/lib/home-*.test.ts`: 56 pass, 0 fail (8 files).
- `bun test src/components/app-shell/app-shell-skeleton.test.tsx src/components/app-shell/shell-placeholder.test.ts`: 15 pass, 0 fail.
- `bun test src/features/copilot/use-copilot.test.tsx src/features/copilot/copilot-page.test.tsx src/features/copilot/chat-messages.test.tsx`: 43 pass, 0 fail.
- No Rust, Python or compose touched; no cargo command run.

**Screenshots**: `docs/plans/web-17-screens/before-*` (6) and `after-*` (18). `*-stubbed` shots use CDP-fulfilled API answers (the QA stack has no chats and no pipelines): `after-1366x800-dark-empty-stubbed`, `after-1366x900-dark-activity-stubbed` (a failed pipeline after Ask why). At 1366×800 the focal dashboard's first tile row ends at 750 px (light and dark), inside the viewport. No horizontal scroll at 390×844 (document and Home's own scroller).

**Decisions taken where the plan was not exact** (please review):
1. The plan calls Customize a dialog; in the code it was an in-place edit mode. Customize is now a menu: Background as three still previews (DESIGN.md §2, a visual choice is a preview) plus "Edit cards and shortcuts" (the old edit mode, unchanged). It is offered even where layouts cannot be saved.
2. Needs attention is a fixed widget, not a card in the catalogue: a person's saved layout from before would not list a new id and the status would vanish for them.
3. "Ask" on attention rows, Ask why, and the empty states now fill the composer (nothing sent); the old status pill's "Ask AI" sent at once. The suggestion chips still send at once, as before.
4. The planner's T5 addition needs the Background known before first paint for the server-rendered shell placeholder: `src/app/layout.tsx` runs a constant script (`HOME_BACKGROUND_SCRIPT`, with the request's CSP nonce, like next-themes) that sets `<html data-home-bg="off">`; the placeholder's band and padding follow it in CSS.
5. Removed with their only users: `recent-card.tsx`, `pipeline-runs-card.tsx`, `start-button.tsx`, `lib/home-recent.ts` and its test, the shortcuts' AI prompts. The page's fade-in and scroll snap are gone (one loading look into the page).

**Not verified**:
- Full `bun run lint` and `bun run test`: not run (owner machine rule; CI runs them).
- iPhone 13 Safari (AC6): not run; only Chrome's 390×844 mobile emulation.
- The hidden-tab pause is in code (`document.hidden` check inside a `requestAnimationFrame` loop) but was not measured.
- CPU numbers are from software rendering on this VM; a real laptop with a GPU will differ.
- Empty and failed-pipeline states were seen only with stubbed API answers; the error states only in unit tests and the QA stack's real "could not check datasets".
- Phone: the three shortcuts and Customize wrap to two lines at 390 px; the composer's text box keeps its two-line minimum (`Textarea` `min-h-16`).

Processes: dev server on 28699 (`bun x next dev` 373591, node 373608, next-server 373654) and headless Chrome on 28690 (373590) stopped by PID; both ports free. `AGENTS.md` rewrites by `next dev` reverted before every commit.
