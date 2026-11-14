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

### Slice 2 (owner QA, 2026-11-15)

9. **T9, the scene is a dim background; no veil, no halo.** Owner: "background ini tu dibuat lebih
   redup aja, sewajarnya background aja. ga sampe ganggu fokus user… shadow lebih gelap di
   tulisan Good morningnya itu jadi kek malah aneh". DESIGN.md §2 now says so as a general rule.
   - Remove the greeting veil (`veilAt`, `data-scene-veil`, the `VEIL_*` constants and the veil
     buckets in `lake-scene.tsx`) and the greeting halo (`GREETING_TEXT` text-shadow and
     `--home-scene-base` in `home-frame.tsx`/`home-page.tsx`). The art is drawn the same
     everywhere; nothing marks the text.
   - Dim the whole scene in both styles (Dither, ASCII) and both phases (day, night, and the
     sun/moon transition between them): move every ink towards the band's base colour by one
     global factor per phase, including the moon, the sun, the whale's eye and the glints. Targets,
     calculated from the colours (WCAG relative luminance) and written in a test:
     - every ink against its phase's base: **at most 2.0:1** (a background, not a picture);
     - the greeting's text colour against every ink and the base: **at least 4.5:1**;
     - the composer, chips and context strip are unchanged.
     The scene keeps its shapes and its motion; only the colours change. The fade into the page
     stays.
   - Check: the contrast test above; the existing scene and Off tests still pass (update any that
     asserted the veil); screenshots `/` at 1366×800 light and dark into
     `docs/plans/web-17-screens/after-dim-*` if a browser is free (else write *not verified*).

10. **T10, light band without the cream; dim to a middle (owner QA 2026-11-15).** Owner, on
    T9's result: "ini tu emang gradient ke kuning kah? dan redup sih tapi jadi ga keliatan".
    - **Light base = the page background.** The day base `#f3eee2` (cream) fading into the
      page reads as a yellow gradient. Use the light theme's page background colour (resolve
      the `--background` token to sRGB; keep a test that pins it) as the day base, so the band
      has no tint of its own and nothing fades. The sun and its glints keep their warm ink. The
      night base (`#061230`) stays. The phase blend between them stays continuous.
    - **Dim less.** T9's factors (0.75 day, 0.8 night) left the ASCII scene invisible and turned
      the navy whale grey in daylight. New window, per style and per phase, at the rest states:
      the most contrasting ink against its base is **between 2.5:1 and 3.0:1**. ASCII glyphs are
      thin, so ASCII may use its own factor and sit at the top of the window; Dither sits nearer
      2.5. With the neutral day base, mixing keeps each ink's hue (no greying).
    - Keep: greeting text **at least 4.5:1** against every ink and the base at the rest states;
      no veil, no halo.
    - Check: update the contrast tests to the window above (both bounds, both styles, both rest
      states, ink-to-base also at the in-between phases with only the upper bound) and to the
      new day base; existing scene tests pass.

11. **T11, a whale that lives in the water (owner QA 2026-11-15).** Owner: "yang aku masih kurang
    suka logo paus kita… entah kurang detail atau kek statis dan ga gerak jadi bikin aneh aja".
    Diagnosis (planner, from the owner's four screenshots at `e1397131`): in `field()` the head is
    a flat disc with near-uniform ink, a hard round eye, and the logo's white gap and band drawn
    as a straight solid bar under it (`y < HZ + 0.03`), so it reads as a sticker on a plinth, not
    a whale in water. It sits in the static sky layer, so it never moves while the water does.
    In ASCII dark the body interior is nearly empty, so it reads as an outline. Keep the logo's
    silhouette (dome, eye on the side; DESIGN.md §5: plain eye, no glow, no pupil, no shine).
    - **Form:** shade the dome as a lit volume: brighter rim on the side facing the sun (day,
      left) or the moon (night, right), darker towards the water, a soft core shadow; the dither
      and the glyph ramp carry the shading. The eye stays plain and flat.
    - **In the water, not on a bar:** remove the straight gap and band. The head's lower edge
      meets the water with a waterline that follows the ripples (no straight edge, DESIGN.md §2),
      a short dark band of wet shadow just above it, and the reflection below broken by the
      ripples as now.
    - **Alive:** a slow bob (the head and its waterline rise and fall a few px, period ~4–5 s, in
      step with the swell); a blink now and then (the eye closes for ~120 ms, every 6–12 s at
      random); a small spout now and then (every ~20–40 s, a short dithered/glyph plume that
      rises and falls in ~1.5 s; DESIGN.md §5 "sesekali menyemburkan air"). These move the
      whale's rows only, within the scene's existing budget: at most 10 fps, paused off-screen
      and in a hidden tab, none under reduced motion (then: no bob, eye open, no spout).
    - **ASCII:** the body is filled with the glyph ramp (not only its edges), so it reads as a
      solid head in both themes.
    - Keep T10's colours and limits: the most contrasting ink against its base stays within
      2.5–3.0:1 (the whale may sit at the top), greeting text ≥4.5:1, tests updated.
    - Check: tests for the waterline not being straight, the blink and spout schedules (seeded,
      no motion under reduced motion), and the contrast limits; screenshots of `/` at 1366×800
      light and dark, Dither and ASCII, plus a frame strip of one spout, into
      `docs/plans/web-17-screens/after-whale-*`. Look at them yourself.

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

### T9 handoff, 2026-11-15 (Sonnet developer agent)

**Commit** `53239526` (`feat(home): WEB-17 T9, the scene is a dim background`; not pushed).

- `lake-scene.tsx`: veil removed (`veilAt`, `Box`, `VEIL_*`, `measureVeil`, the veil buckets and steps; the resize observer now watches only the canvas). New `DIM = { day: 0.75, night: 0.8 }` and exported `sceneInks(phase)`: every ink (moon, sun, whale's eye and glints included) is blended day to night, then moved towards the blended base by `DIM` interpolated on the phase, so a theme change stays continuous. Both styles use it. Shapes and motion are unchanged.
- `home-frame.tsx` / `home-page.tsx`: `GREETING_TEXT`, `--home-scene-base` and `data-scene-veil` are gone; the greeting is plain text.
- `lake-scene.test.ts`: WCAG contrast test. Ink against base at most 2.0:1 at 7 phases from day to night (worst 1.76 day, 1.74 night). Greeting text (theme foreground as sRGB, #0b0f11 and #f8fbfc) against every ink and the base at least 4.5:1 at rest (worst 9.47 day, 10.23 night).
- Not asserted: the midpoints of a theme change against the text. The text takes the new theme's colour at once while the scene eases for 1.5 s, so that moment was never readable by design.

**Checks:** typecheck clean; `bun test src/features/home src/lib/home-*.test.ts` 63 pass, 0 fail; eslint on the 4 changed files clean.

**Not verified:** no screenshots (`after-dim-*` not taken) and no browser; how dim it looks is for the owner to judge.

### T10 handoff, 2026-11-15 (Sonnet developer agent)

**Commit** `e1397131` (`feat(home): WEB-17 T10, a neutral light band and a middle dim`; not pushed).

- Day base is `#ffffff`, the light `--background` (`oklch(1 0 0)`); the test reads `design-system/tokens/colors.css` and pins both. Night base `#061230` unchanged. `BASE` is exported.
- `DIM` is per style: Dither day 0.62, night 0.69; ASCII day 0.57, night 0.65 (interpolated on the phase). `sceneInks(phase, style)`; the scene passes its variant.
- Measured ink against base at rest, best to worst (min to max): Dither day 1.43 to 2.53, night 1.36 to 2.55; ASCII day 1.50 to 2.93, night 1.43 to 2.89. In between (0.25, 0.5, 0.75): at most 1.78, 1.19, 1.89 (Dither) and 1.95, 1.22, 2.06 (ASCII).
- Greeting text against every ink and the base at rest: worst 6.14:1 (ASCII night); day 6.58 to 7.60.
- Tests: both bounds (2.5 to 3.0) at rest for both styles, ASCII above Dither, upper bound 3.0 at in-between phases, text 4.5, the pin, and the day whale still blue.
- Note: the navy whale at day is a pale periwinkle, not navy, because the window caps its contrast with white at about 2.5 to 2.9. It is blue-tinted (not neutral grey), but it is light.

**Checks:** typecheck clean; `bun test src/features/home src/lib/home-*.test.ts` 65 pass, 0 fail; eslint on the 2 changed files clean.

**Not verified:** no browser, no screenshots.

### T11 handoff, 2026-11-15 (Opus developer agent)

**Commit** `777dfe4b` (`feat(home): WEB-17 T11, a whale that lives in the water`; not pushed).
Only `lake-scene.tsx`, its test, and `docs/plans/web-17-screens/after-whale-*` changed.

- **Form.** The dome is shaded as a sphere. By day it is lit from the sun's side (upper left), by night
  from the moon's (upper right), and during a theme change the light swings over the top. The shading
  is diffuse light with a soft core shadow on the far side, a brighter rim on the near side, and a wet
  shadow over the last ~14 px above the water (`headLight`). The dither and the glyph ramp carry it.
  The old hard rim ring and sheen are gone. The eye stays plain and flat and is now solid
  (light 0.99, above the highest Bayer threshold): at 0.92 the dither punched two dark dots into it
  that read as a pupil.
- **Waterline.** The straight gap and band (`y < HZ + 0.03`) are removed. The head ends at
  `waterlineAt(x, t, pose)`, which takes 0.6 of the bob and adds two ripple waves driven like the
  water's swell (±5.6 px on the 400 px band). In a trough, the head continues below `HZ`; a crest
  covers it above `HZ`. Just under the line is a broken glint (the mark's white gap, now wet). The
  reflection is taken about the line it rides, wavers wider with depth and thins out instead of ending
  on a straight edge.
- **Motion (`whalePose`, `eventAt`).**
  - Bob: ±0.009 of the height (±3.6 px), period 4.6 s, sine.
  - Blink: the eye shuts to a slit at its lower lid for 120 ms. Slots of 9 s ± 1.5 s, so blinks
    come 6 to 12 s apart.
  - Spout: 1.5 s. A stem shoots up (ease-out, 0.34 R), then a crown opens wider as it falls back
    and fades. Slots of 30 s ± 5 s, so spouts come 20 to 40 s apart.
  - The schedule is seeded and constant-time (it looks only at the nearby slots). Seed 0 is used,
    so the first spout comes at scene time 10 to 20 s, about 3 to 13 s after load.
  - Under reduced motion and in the Background previews, `poseFor(t, true)` is `REST`: no bob, eye
    open, no spout.
  - Only the whale's box is recomputed per frame (`whaleBox`, about 56 × 48 cells at 1110 × 400),
    inside the same ≤10 fps loop with the same gating. ASCII leaves that box out of the cached sky
    layer and draws it each frame.
- **Spray ink** is region 8. It uses the water's blue by day and the moonlight's cream by night,
  both already in the palette, so it adds no colour. By day its density is 1.3×, so it reads as a
  plume against the sky's dots.
- **ASCII** fills the head from the ramp's lightest step (`0.18 + 0.82 × ink`). The dark side is now
  `.`/`,`/`:` instead of empty.
- **Contrast** (WCAG; `DIM` and the inks are unchanged from T10). Ink against base at rest, as the
  maximum, with the whale's own value in brackets:
  - Dither: day 2.53 (whale 2.53), night 2.55 (whale 2.08).
  - ASCII: day 2.93 (whale 2.93), night 2.89 (whale 2.32).
  - In between, at phases 0.25, 0.5 and 0.75: at most 2.06.
  - The spray matches the existing inks (at most 2.89).
  - Greeting text against every ink and the base, at its worst: 7.60 (Dither day), 6.96 (Dither
    night), 6.58 (ASCII day), 6.14 (ASCII night).
- **Cost.** Measured in headless Chrome 148 on this VM, `next dev`, 1366×800, Dark Dither, using
  `Performance.getMetrics` over 10 s, against HEAD swapped in on the same server:
  - Main thread busy: 12.8–13.1 % (HEAD 10.9–11.0 %). Script: 9.8–10.1 % (HEAD 8.0–8.1 %).
  - ASCII: 13.1 % busy.
  - The first build cost +3 points more, because V8 stopped inlining `inkAt` into the dither loop.
    The ASCII fill moved to `glyphRows`, and the eye now returns a float pair (an all-integer
    `[1, 3]` made the destructuring polymorphic). The CPU profile confirmed both fixes.
  - Measured on the canvas: reduced motion gives 1 distinct whale-box frame in 4.2 s; with motion,
    6 of 6 frames differ.

**Checks** (each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`,
one at a time, after `free -m` showed ≥ 3 GB available):
- `bun run typecheck`: exit 0.
- `bun test src/features/home src/lib/home-*.test.ts`: 73 pass, 0 fail (9 files). That is 65 + 8
  new T11 tests:
  - the waterline is not straight (≥ 2 cells) and moves;
  - no bar under the head;
  - the lit side follows the phase, and the wet shadow is darker;
  - blink and spout gaps and lengths for 3 seeds, and the seed is deterministic;
  - the spout rises, then falls, and stays ≥ 14 px inside the band;
  - no motion under reduced motion;
  - the whale and the spout never sit behind the greeting at any pose;
  - the spray adds no colour.
- `bun x eslint src/features/home/lake-scene.tsx src/features/home/lake-scene.test.ts`: exit 0.

**Screenshots** are in `docs/plans/web-17-screens/`. Each was taken on the QA API, logged in as the
owner account:
- `after-whale-1366x800-{light,dark}-{dither,ascii}.png`;
- a 2× crop of the whale for each (`*-whale.png`);
- `after-whale-1366x800-dark-dither-spout-strip.png`, 8 frames from before the spout to 1.0 s.

The developer's own judgement of them:
- Day: the head reads as a lit volume. The left side is lighter, the right side and the bottom are
  denser, and the line where it meets the water is visibly wavy, with white glints under it. The
  reflection no longer forms a block. The whale is still the pale periwinkle that T10 noted (not
  navy), because of T10's colour window.
- Night: the moon side is lit and the far side falls into shadow against the sky. The eye is solid.
  The whole picture stays dim and calm, and the composer stays the focus.
- ASCII: the head is solid in both themes. Where a trough dips below the mean waterline, a lone
  `@@` cell can appear in the first water row; it reads as a drip.
- The strip shows the stem, the crown opening, its fall, and the head's bob between frames.

**Not verified:**
- Full `bun run lint` and `bun run test`: not run (machine rule).
- Phone width: no screenshot at 390 px. The test covers the greeting box on a phone.
- iPhone Safari: not checked.
- Light-theme spout in the browser: only rendered offline with the same `field()`.
- The hidden-tab pause: not measured; the gating is unchanged from slice 1.
- CPU numbers come from software rendering under `next dev`, not a production build.
- The blink is 120 ms against a ≥100 ms frame step, so a late frame can skip one blink.

Processes: the dev server on 28699 (bash 2211856, bun 2211860, node 2211862, next-server 2211933)
and headless Chrome on 28690 (2212326, 2212329) were stopped by PID, and both ports are free. Before
the commit, `next dev`'s rewrite of `AGENTS.md` was reverted.

## Micro-interaction audit (emil), 2026-11-14

Owner request (2026-11-14): audit the revamped pages' micro-animations with the
`emil-design-eng` checklist; DESIGN.md wins (150–250 ms, eased, never linear;
motion with a reason; everything stops under reduced motion). Opus developer
agent. Motion only: no layout, colour or copy changes; roles and accessible
names unchanged. `lake-scene.tsx` is not touched (another session works on it).

The shared part (easing tokens, the global reduced-motion rule, tooltips with
a first delay and instant follow-ups, the button press, menus, popovers,
dialogs, sheets, `useSlidingIndicator`) is the same commit as on BI-61,
cherry-picked: its table is in the BI-61 plan.

| Element | Now | Skill / DESIGN.md | Change |
| --- | --- | --- | --- |
| `EASE` (chips, widget rows, links, Ask why) | Lists `transform`, so every `active:scale-*` press jumped (Tailwind v4 sets `scale`) | Name the property; strong ease-out | `scale,rotate,translate`, `ease-ui` (the same hunk as BI-61, so the merge is clean) |
| Background switch Dither ↔ ASCII | The band redraws the other art in one frame | Owner: cross-fade, never jump | The new art mounts as a second band over the old and fades in (300 ms); the old one goes once the new one is opaque. Off still fades out in place |
| The scene's first appearance | Art appears in one frame over the base colour | Prevent jarring changes | Only the art fades in (400 ms) over the base colour the server painted, so the first frame never flashes the plain page |
| Background previews (Customize) | The ring jumps to the picked picture | Eased state change; press feedback | Ring eases (200 ms), the picture gives 0.97 under the press |
| Ask / Build switch | The raised fill jumps; `transition-colors` | Sliding indicator; press | One pill that slides (220 ms `ease-move`), press 0.97 |
| Context strip under the composer | Text swaps in one frame on Ask / Build | Crossfade | The new line fades in (200 ms) |
| Composer box | `transition-all` | Never `all` | Border, background and shadow named, `ease-ui` |
| "Ask why" filling the composer | Text appears; a box out of view jumps into view (`focus()`) | A small, meaningful cue | The box's edge lights up in primary and eases back (650 ms); out of view, it glides into the middle (instant under reduced motion) |
| Cycling example placeholder | 250 ms both ways, default curve | Exit faster than enter | In 250 ms / out 150 ms, `ease-ui` |
| Send, Stop, Tools | Colour only | Press feedback | Press 0.95 (Send only while enabled), Tools 0.97 |
| Suggestion chips, widget rows, Open links | Press scale written but not animated | Press 0.97 / 0.99, eased | Animated now (`EASE`) |
| Focal dashboard tiles | Title hover not eased; chart replaces its placeholder in one frame | Fill in place | Title colour eases; the chart fades in (200 ms) |
| Widget rows landing | Rows replace placeholder rows in one frame | Same | Rows fade in; the placeholder rows (`aria-hidden`) do not, so loading never blinks |
