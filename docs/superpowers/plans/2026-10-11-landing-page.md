# Lakehouse landing page (`/landing`) — Implementation Plan

**Status:** asked for by the product owner on 2026-10-11 ("the best landing page
for the lakehouse product… different from our other product pages… modern and
really creative, with 3D and on-scroll effects, metamask.io level"). Written by
the planner (Claude Opus) for a developer agent (non-Opus), under the role
split in `AGENTS.md`. The planner writes no product code. Backlog `WEB-1`.

**Base:** `main` at `9cff2c2`.

---

## 1. The idea in one paragraph

The RantAI Lakehouse logo is a creature: a navy dome with one white eye,
peeking above a horizon line over a blue lake (`public/logo-light.png`). The
page turns it into a 3D mascot, the way metamask.io turned its fox into one.
It floats in a dark 3D lake at night, its eye follows the cursor, and as the
visitor scrolls the camera **dives below the surface**: through falling data
(connectors), down through three glowing depths (Bronze, Silver, Gold), past
the guard at the bottom (governance), and back up to the shore where the
dashboards live. One continuous story: *data falls in, sinks into layers,
and comes back up as answers.*

## 2. Decisions already made

1. **Route and access.** `src/app/landing/page.tsx`. Add `"/landing"` to
   `PUBLIC_PREFIXES` in `src/features/auth/auth-provider.tsx:21`, so it
   renders without the console chrome and without a sign-in redirect
   (`src/components/app-shell/app-frame.tsx:32` already skips the frame for
   public paths). Nothing else in the auth path changes.
2. **Different from the sister sites.** rantai.dev, agents.rantai.dev and
   llmops.rantai.dev are light, card-and-screenshot pages in Geist / Hanken
   Grotesk / Poppins with a corporate blue. This page is **dark, immersive,
   scroll-driven and 3D**. It shares only the family links in the footer and
   the RantAI name.
3. **Palette (from the logo, sampled):** abyss `#020514`, navy `#050A30`
   (logo dome, also `theme-color` in `src/app/layout.tsx`), lake `#44A6FA`
   (logo water), foam `#E8F4FF`. Depth accents for the three layers only:
   bronze `#C8834A`, silver `#B9C6D3`, gold `#F2C14E`. No other hues.
4. **Type.** Display: **Instrument Serif** italic for big emotional lines
   (contrast with the sans sister sites), loaded with `next/font/google`.
   Body and UI: Geist (already in the root layout). Mono labels: Geist Mono.
5. **3D engine.** Add **`three`** (MIT) and `@types/three` (dev). Plain
   three.js inside a client component with a `useEffect`-owned renderer. No
   React Three Fiber, no Lenis, no GSAP: one new runtime dependency only.
   Scroll and section animation use **`motion`** (already a dependency:
   `useScroll`, `useTransform`, `useSpring`, `motion.div`).
6. **Honesty (principle 2).** Every capability named on the page is one
   `docs/core/PRODUCT.md` §2 marks Have, or Partial worded as what it does
   today. **No** customer logos, user counts, uptime figures, benchmark
   numbers, testimonials or "trusted by". No pricing. The copy in section 5
   is the only copy; do not invent more.
7. **Calls to action.** Primary: "Open the console" → `/login`. Secondary:
   "Talk to us" → `mailto:contact@rantai.dev?subject=RantAI%20Lakehouse`.
   Tertiary: "Read the source" →
   `https://github.com/RantAI-dev/RantAI-Lakehouse`.
8. **Accessibility and fallbacks.**
   - `prefers-reduced-motion: reduce` → no camera dive, no parallax, no
     autoplay loops; sections fade in only; the mascot is still and its eye
     does not track.
   - No WebGL (or context lost) → the hero shows the logo as an inline SVG
     over a CSS gradient lake, and the page still reads end to end.
   - Every section works without the canvas: text is real DOM, never drawn
     into WebGL.
   - Keyboard: skip link, visible focus rings, buttons are real `<a>`.
   - Colour contrast of body text ≥ 4.5:1 on its background.
9. **Performance budget.** Device pixel ratio capped at 1.75 (1.25 on
   phones); the render loop stops when the canvas is off screen
   (IntersectionObserver) and when the tab is hidden; particle counts halve
   below 768 px; `three` is loaded with `next/dynamic` (`ssr: false`) so the
   first paint is the DOM hero, not a blank canvas. Dispose geometries,
   materials and the renderer on unmount.
10. **Mobile.** Below 768 px the eye follows device tilt if permission is
    already granted, otherwise it idles (slow look-around). All sections
    stack to one column. No horizontal page scroll at 390 px.

## 3. Anchors (verified at `9cff2c2`)

| What | Where |
| --- | --- |
| Public routes | `src/features/auth/auth-provider.tsx:21` `PUBLIC_PREFIXES` |
| Frame skip for public routes | `src/components/app-shell/app-frame.tsx:32` |
| Root layout, fonts, theme-color | `src/app/layout.tsx` |
| Logo (colour) | `public/logo-light.png` (512×512; dome `#050A30`, water `#44A6FA`, eye and horizon are transparent cut-outs) |
| Logo (for dark backgrounds) | `public/logo-dark.png` |
| `motion` version | `package.json` `"motion": "^13.4.0"` |
| Next.js | `16.1.6` (App Router, React 19.3) |

## 4. The page, section by section

Total scroll length about 9–10 viewport heights. Sections are full-bleed;
text is centred in a max-width column of 1120 px with a 24 px gutter (16 px
on phones).

### S0. Top bar (fixed)
Logo mark (24 px) + "RantAI Lakehouse" wordmark left. Right: "Product",
"How it works", "Governance", "Open source" (anchor links), then a small
pill button "Open the console". Transparent over the hero, gains a
backdrop-blur and a 1 px foam/10% bottom border after 40 px of scroll.

### S1. Hero — "the creature in the lake" (100 vh, pinned for +60 vh)
- **3D scene.** A night lake: a large plane with a custom water shader
  (scrolling normal noise from two layered simplex/value-noise functions
  computed in the shader, fresnel toward the horizon, a soft moon reflection
  streak in lake blue). Fog from navy to abyss. Faint drifting star points
  above the horizon (≤ 600, ≤ 300 on phones).
- **The mascot.** Built from primitives so it matches the logo exactly: a
  navy hemisphere-capped dome (slightly squashed, a `MeshPhysicalMaterial`
  with clearcoat so it catches the moonlight), half submerged, bobbing on a
  slow sine (amplitude 0.06, period about 4 s) with a tiny tilt. One white
  eye (ellipsoid) on the right side of the face, positioned as in the logo.
  **The eye follows the pointer** (eased, clamped to a cone so it never
  leaves the face); every 4–7 s it **blinks** (scale-Y to 0.1 and back in
  140 ms). A soft ring ripple expands from the waterline each bob.
- **Interaction.** Moving the pointer also turns the whole dome a few
  degrees toward it (like the MetaMask fox). Clicking the dome makes it
  dip under and pop back up with a splash ring.
- **Copy (DOM, over the canvas, left-aligned on desktop, centred on phones).**
  - Eyebrow (mono, lake): `RANTAI LAKEHOUSE`
  - Headline (Instrument Serif italic, 72–120 px fluid): **Your data has a
    home now.**
  - Sub (Geist, 18–20 px, foam/80%): "One lakehouse with dashboards built in.
    Bring every source in, keep it in open tables, and ask for answers in
    plain language."
  - Buttons: "Open the console" (solid lake, abyss text) and "Talk to us"
    (ghost, foam border).
  - Scroll cue at the bottom: a thin line with a droplet sliding down it,
    label "Scroll to dive".
- **Scroll.** Over the pinned +60 vh the copy fades and lifts away, the
  camera tilts down and **dives through the water surface**: the surface
  becomes a rippling ceiling seen from below (render the plane double-sided
  and swap to an "underwater" colour ramp when the camera is under it), the
  scene tints deep navy, light shafts (additive planes) fall from above. The
  mascot stays near the surface, looking down at the visitor as they sink.

### S2. "Everything flows in" — sources (about 150 vh)
- Background: the underwater canvas continues (the hero canvas is
  `position: fixed` behind S1–S3 and its uniforms are driven by overall
  scroll progress; it fades out at the end of S3).
- Content: headline (serif italic) **Everything flows in.** Sub: "Databases,
  files, streams and APIs land in one place, on a schedule or as they
  change."
- The sources fall as **glowing droplets** through the water, each a pill
  label that drifts down and settles into a "sediment" grid on scroll. The
  list, all built today: PostgreSQL, MySQL, SQL Server, MongoDB, REST APIs,
  Kafka, CSV and TSV files, Excel workbooks, Parquet files. One pill per
  item, mono text. (SFTP and S3-compatible storage stay off until `VER-1`
  and `SRC-6` are done; review round 1.)
- A small caption under the grid: "Live change capture for PostgreSQL. File
  upload straight from the console." (both are built).

### S3. "Three depths" — the medallion layers (pinned, about 250 vh)
The signature scroll moment.
- A **3D stack of three translucent slabs** (rounded boxes, glass-like
  `MeshPhysicalMaterial` with transmission, each with its own accent glow):
  Bronze at the top, Silver in the middle, Gold at the bottom. As the
  visitor scrolls, the camera sinks past each slab in turn; the active slab
  lights up, separates slightly from the others, and small particles
  ("rows") stream down from the slab above into it, getting fewer and more
  ordered at each depth (noisy at Bronze, aligned in rows at Silver, few and
  bright at Gold).
- A sticky text panel beside the stack changes with the active depth:
  - **Bronze — Raw, kept as it came.** "Every source lands untouched in open
    Apache Iceberg tables, so nothing is lost and nothing is locked in."
  - **Silver — Cleaned and joined.** "Pipelines with schedules, dependencies,
    retries and version history clean the raw data into tables you can
    trust."
  - **Gold — Ready for answers.** "Curated tables built for the questions
    your business asks, and published as open tables when you switch it on."
- A depth gauge on the left edge (mono): `−10 m BRONZE · −40 m SILVER ·
  −90 m GOLD`, with a marker that tracks scroll.

### S4. "Ask, don't dig" — the assistant (about 140 vh)
- The canvas has faded; background becomes a deep navy with a subtle
  caustic light pattern (CSS: two animated radial gradients, reduced-motion
  off).
- Headline: **Ask, don't dig.** Sub: "The assistant works with the same
  tools as the console. It answers questions, builds charts and dashboards,
  and runs pipelines. Anything risky waits for a person to approve it."
- A scripted, scroll-scrubbed **chat mock** (DOM, not a screenshot): the
  user bubble types "Which region had the most visitors last quarter?"; a
  thinking shimmer; the answer bubble appears with a small bar chart that
  grows (motion), then a second exchange: "Pause the nightly sales
  pipeline." → an **approval card** slides in: "Waiting for approval · Pause
  pipeline `sales_nightly`" with "Approve" / "Decline" buttons (decorative,
  `aria-hidden` on the fake controls, with a visually hidden sentence
  describing the demo). Each step is tied to scroll progress, so scrolling
  back rewinds it.
- Region names and numbers in the mock are obviously illustrative; label the
  mock "Illustration" in a small mono caption.

### S5. "Dashboards live on the shore" — BI (about 160 vh, horizontal)
- We have surfaced: the background warms slightly to a dusk gradient (navy →
  `#0B1A4A` → a thin lake-blue horizon line), the mascot's silhouette sits
  on the horizon at the far right, tiny.
- Headline: **Answers come back up as dashboards.** Sub: "About two dozen
  chart types, maps included. Filter, drill down, share by link, embed in
  your own site, and get an alert when a number crosses a line."
- A **horizontal scroll-jacked rail** (vertical scroll drives `x` of a
  wide row via `useScroll` + sticky container) of 5 dashboard tiles drawn in
  SVG/DOM (no screenshots): a line chart, a choropleth-style map of dots, a
  sankey, a KPI tile with a sparkline, a calendar heatmap. Each tile tilts
  in 3D toward the cursor on hover (`perspective` + rotateX/Y, max 8°) and
  has a glossy sheen that follows the pointer.

### S6. "Guarded by default" — governance (about 120 vh)
- Headline: **Guarded by default.** Sub: "Roles decide who sees what.
  Sensitive columns are classified and masked for everyone who should not
  see them, and lineage shows which tables each table is built from."
- An interactive **masking table** (DOM): 5 rows × 4 columns (name, email,
  city, amount). A toggle "View as: Analyst / Admin". As Analyst, the email
  and name cells **scramble** (random glyph cycling for 400 ms) into masked
  values (`a•••@•••.com`); switching to Admin unscrambles them. Starts in
  Admin and flips to Analyst once when the table scrolls into view.
- Three small facts under it, each a mono label + one line: "Masking proven
  end to end in an acceptance gate", "Assistant actions logged; risky ones
  wait for approval", "Lineage from raw table to curated table".

### S7. "Runs where you do" — open source and open formats (about 100 vh)
- Headline: **Yours to keep.** Three large cards in a row (stacked on
  phones), each with a big outlined numeral and a 3D-tilt on hover:
  1. **Open source.** "AGPL-3.0. Read every line, run it yourself."
  2. **Open tables.** "Raw data in Apache Iceberg. Any engine that reads
     Iceberg can read it."
  3. **Your servers or ours.** "Install it on your own infrastructure, or
     talk to us about running it for you." (Do not claim a hosted service
     exists; this sentence offers a conversation.)
- Under the cards: "Read the source" link to GitHub.

### S8. Final call (100 vh)
- Back to the night lake look (CSS gradient + a small second canvas or the
  SVG logo, whichever is lighter). The mascot, large and centred, half
  submerged, eye looking up at the headline.
- Headline (serif italic): **Come on in. The data's fine.**
- Buttons: "Open the console", "Talk to us".

### S9. Footer
- Left: logo + "RantAI Lakehouse. One lakehouse with dashboards built in."
- Columns: **Product** (anchors), **RantAI** (rantai.dev, RantAI Agents →
  agents.rantai.dev, RantAI LLMOps → llmops.rantai.dev), **Connect**
  (GitHub, contact@rantai.dev).
- "© 2026 RantAI. Depok, West Java, Indonesia."

## 5. Tasks (one commit each, in order)

| # | Task | Acceptance check |
| --- | --- | --- |
| L1 | Add `three` and `@types/three`; add `/landing` to `PUBLIC_PREFIXES`; create `src/app/landing/page.tsx` with metadata (title "RantAI Lakehouse — your data has a home", description from S1 sub, `themeColor` `#020514`) and the static DOM for S0–S9 with final copy, layout and fonts, no 3D yet | `/landing` loads signed out with no console chrome and no redirect; all copy present; no horizontal scroll at 390 px |
| L2 | Hero 3D scene: water shader, stars, mascot with eye tracking, blink, bob, ripple, click-to-dip; WebGL fallback; reduced-motion handling; off-screen and hidden-tab pause | Eye follows the cursor; with WebGL disabled the SVG logo hero shows; with reduced motion the scene is still |
| L3 | Scroll-driven dive and underwater state (S1→S2), falling source droplets settling into the grid | Scrolling down dives below the surface; scrolling up returns; droplets settle |
| L4 | Medallion stack (S3) with active-layer glow, particle streams, sticky copy and depth gauge | Each layer becomes active in turn as you scroll; copy and gauge stay in sync |
| L5 | Chat mock (S4) scrubbed by scroll; dashboards rail (S5) with hover tilt and sheen | Scrolling back rewinds the chat; rail moves horizontally with vertical scroll |
| L6 | Masking table (S6), open cards (S7), final call (S8), footer (S9), top bar behaviour | Toggle scrambles and unscrambles; links point where §2.7 says |
| L7 | Polish and checks: Lighthouse-style pass (no layout shift on load, images sized), dispose on unmount, focus rings, contrast, phone layout | Checks in §6 pass |

Put the landing code in `src/features/landing/` (kebab-case files,
`"use client"` first line where needed) with the page in `src/app/landing/`.
The feature must not import from `@/services` (it calls no API).

## 6. Verification before handoff

```bash
bun run typecheck && bun run lint && bun run test
git status
```

Plus, by hand with Playwright (Chromium at `/opt/pw-browsers/chromium`)
against `bun run dev`:
- Screenshots at 1440×900 and 390×844 at scroll 0%, 15%, 35%, 55%, 75%, 100%.
- No console errors; no horizontal overflow at 390 px
  (`document.documentElement.scrollWidth <= innerWidth`).
- With `--disable-webgl` the hero fallback renders.
- With `reducedMotion: 'reduce'` no camera dive happens.

## 7. Out of scope

- Pricing, sign-up, analytics or tracking scripts, cookie banners.
- Any change to the console pages or the API.
- A new domain or deployment: the page ships at `/landing` in this app.

## 8. Handoff (developer)

Branch `claude/gracious-albattani-vw5wsb`, base `main` at `9cff2c2`, plan at `80db022`. One commit per task, in plan order. Nothing pushed, no PR.

### Commits

| Task | Hash | Subject |
| --- | --- | --- |
| L1 | `226f37c` | feat(landing): add the /landing page with its static sections (L1) |
| L2 | `b497f19` | feat(landing): hero 3D lake with the mascot, eye tracking and fallbacks (L2) |
| L3 | `0b284bb` | feat(landing): scroll-driven dive, underwater look and falling sources (L3) |
| L4 | `bd0dc3e` | feat(landing): medallion stack with active-layer glow, streams and depth gauge (L4) |
| L5 | `85c9e9f` | feat(landing): scrubbed assistant chat and the horizontal dashboards rail (L5) |
| L6 | `ad590f5` | feat(landing): masking table, open cards, final call, footer and top bar (L6) |
| L7 | `9bfb888` | feat(landing): polish and checks (L7) |

### Commands run, on the final code (commit `9bfb888`, before this handoff commit)

All in the foreground from `/home/user/RantAI-Lakehouse`.

- `bun run typecheck` -> exit 0, no output after the `tsc --noEmit` banner.
- `bun run lint` -> `6 problems (0 errors, 6 warnings)`. All six are in files this branch does not touch (`alerts-page.tsx`, `open-format-card.test.tsx`, `use-data-table.ts`, `dashboard-specs.ts` and two more); none is under `src/features/landing`, `src/app/landing` or `src/features/auth` (`bun --bun eslint` on those three paths is silent).
- `bun run test` -> `954 pass, 1 skip, 0 fail, 2352 expect() calls, 955 tests across 105 files` (111.5 s). The known flaky `ConnectorIngestPanel layout` test did not fail in either of the two full runs. New tests: `scroll-timeline.test.ts` (12), `scramble.test.ts` (7), `auth-provider.test.ts` (3).
- `bun run build` -> exit 0, `Compiled successfully in 26.6s`; `/landing` is listed as `○` (prerendered static).
- `git status` -> clean after each commit (no `__pycache__`, `.next`, generated files; `bun.lock` and `package.json` carry the two new packages).
- No cargo, Python or compose command was run: this branch touches none of them.

### Playwright checks (§6), Chromium at `/opt/pw-browsers/chromium`, software GL (SwiftShader), against `bun run dev`

- Screenshots at 1440x900 and 390x844 at scroll 0, 15, 35, 55, 75, 100 %: `final-desktop-*.png`, `final-phone-*.png`. Also `final-reduced-desktop-*.png` (reduced motion) and `final-nowebgl-desktop-*.png` (WebGL off).
- Folder: `/tmp/claude-0/-home-user-RantAI-Lakehouse/d09405be-e728-58d5-b5e1-7eaf371f0bbf/scratchpad/landing-shots/` (the scripts `shot.js` and `checks.js` are there too). The desktop 15 / 55 / 75 / 100 % frames were taken one tweak before the last commit (the depth stack's anchor moved afterwards); the 35 % frame is from the final code.
- Console: no errors and no warnings from the page, at 1440 and 390, with WebGL, without WebGL, with WebGL lost mid-session, and with reduced motion. Filtered as environment, not page: the dev server's `/api/auth/me` proxy failure (the root `AuthProvider` asks for a session on every route and no API is running here; the landing page itself calls nothing), the `react-grab` dev tool's version check and the Google Fonts fetch (the sandbox has no trusted TLS for them).
- Horizontal overflow: `scrollWidth <= innerWidth` is true at 1440 and at 390.
- Layout shift on load (PerformanceObserver `layout-shift`): 0 at both sizes.
- No console chrome and no sign-in redirect: no sidebar element, `location.pathname` stays `/landing`.
- WebGL off (`--disable-3d-apis`): the SVG hero over the CSS lake renders, no canvas element. WebGL lost mid-session (`WEBGL_lose_context`): the canvas is removed and the SVG hero comes back (opacity 1).
- Reduced motion: no pin, no dive; the hero shows one still frame of the lake, the sections are plain blocks, the depths are three cards.
- Off screen: 64 draw calls in 3 s while the zone is visible, 0 in 3 s with the page scrolled to the footer.
- Interaction: the eye follows the pointer (left, up and down checked in screenshots), the masking table flips to Analyst when it scrolls into view and the toggle scrambles and unscrambles it.

### Not done or not verified

- **Real GPU performance.** Everything ran on software GL at 2 to 4 frames per second. The 60 fps goal, the adaptive pixel-ratio step-down and the 1.75 / 1.25 caps are implemented but their effect was not measured on a GPU or a phone.
- **Browsers and devices other than Chromium.** No Safari, Firefox, iOS or Android run. The phone tilt path (`deviceorientation`) and the idle look-around are untested on a device; iOS is not asked for permission (plan 2.10: "if permission is already granted").
- **Hidden-tab pause** is implemented (`visibilitychange`) but cannot be exercised in headless Chromium; the off-screen pause was measured.
- **Unmount disposal** was exercised by React StrictMode's double mount in dev and by the context-lost path, not by a client-side route change (the console CTA is a plain `<a>`, so it reloads). `dispose()` frees the geometries, materials, the environment target and the renderer by reading the code, not by a memory measurement.
- **Lighthouse** was not run (no Lighthouse in the sandbox); the checks above stand in for "no layout shift, images sized, focus, contrast, phone layout". There are no `<img>` elements; the artwork is SVG and canvas.
- **Contrast** was computed by hand for the text tokens (smallest: foam at 50 % on abyss, about 5:1), not with a tool; the hero copy sits on a dark scrim over the canvas.
- `CHANGELOG.md` and the `docs/core/` pages were not touched (the plan lists them for the planner).

### Deviations from the plan, and why

1. **Section lengths.** Hero 150 vh (100 vh stage pinned for 50 vh, then it scrolls away while the dive finishes; plan: pinned +60 vh), sources 230 vh (plan about 150), depths 300 vh (about 250), assistant 260 vh (about 140), dashboards 100 vh plus the measured sideways travel (about 210 to 240 vh; plan about 160). The scrubbed chat and the three slabs need that much scroll to read; shorter versions raced past. Nothing else about the sections changed.
2. **Scroll progress is `use-section-progress.ts`, not motion's `useScroll`.** motion 13 hands scroll-linked opacity and filter to the native ViewTimeline; with a tall section holding a sticky stage, opacity and blur stayed at their first values while the transform moved (measured, see the L3 commit body). The hook is plain `scrollY` arithmetic and the same numbers the 3D engine uses.
3. **Copy added around the plan's copy** (none of it claims a capability): the assistant's answer line "Region North had the most visitors last quarter." and four made-up bar values; the five dashboard tile titles and the KPI text ("24.6k", "+8.4% vs last week"); "Illustration · fictional people" above the masking table; a "Three depths" eyebrow, "View as" labels, screen-reader captions and the skip link. All made-up figures sit inside blocks labelled as illustrations.
4. **The approval card is lake blue, not gold.** Plan 2.3 reserves the three metal accents for the depths. The pulse keyframe changed with it.
5. **`/landing` carries two `theme-color` tags.** The page's `viewport.themeColor` (`#020514`, as the plan asks) is added to the root layout's static `<meta name="theme-color" content="#050A30">`, which the plan says not to change.
6. **Top bar on phones** shows the wordmark and the console button only; the four anchors are hidden below 768 px (they do not fit beside the button).
7. **Hero headline** uses `clamp(4.5rem, 9vw, 7.5rem)`: 72 px minimum, 120 px maximum, as the plan says, so at 390 px it is 72 px.
8. **Camera hop at the surface.** Within 0.14 units of the water plane the near plane clips it away and the view opens onto the submerged half, so `avoidSurface()` moves the camera across that band in one step. It is covered by a test and happens at the moment the picture changes from sky to ceiling.
9. **No front fill light on the mascot** (a directional light in front of a clearcoated dome shows as a hot spot on its face); the face is lit by ambient, a hemisphere bounce and the eye's halo.

### For the reviewer

- The sky dome draws the under-water colour itself: three does not tone-map the clear colour, so the water's fog did not match it (L3 commit body).
- `useReducedMotionSafe()` replaces motion's `useReducedMotion()` in the sections: the latter reads the media query during the first client render and reported a hydration mismatch for reduced-motion visitors.
- The dashboard tiles' pseudo-random values use integer maths only: the server renders with a different JavaScript engine than the browser hydrates with.
- `package-lock.json` exists beside `bun.lock` and was not regenerated; only `bun.lock` and `package.json` carry `three` and `@types/three`.

### Round 1 fixes

One commit per finding, each citing it at the fix site and in the body.

| Finding | Hash | What changed |
| --- | --- | --- |
| BLOCKER 1 | `2f9b1c8` | `landing-copy.ts`: SFTP and S3-compatible storage removed (nine pills, laid out 6 + 3); governance sub, the Lineage fact and the Audit fact reworded as the review gave them. The assistant's screen-reader caption does not repeat any of those lines, so nothing else changed. |
| SHOULD-FIX 1 | `4c76316` | Pills ease out with no overshoot (the old one overshot about 100 px onto the caption); grid narrowed to 760 px, caption margin up. |
| SHOULD-FIX 2 | `d6c35cb` | Engine sets canvas opacity to `1 - smoothstep(0.82, 1, depthProgress)`; the assistant stage opens with an abyss gradient drawn above its glow blobs (they were clipped to a visible edge at the stage top). |
| SHOULD-FIX 3 | `3e6d8f4` | Rim light is the open dome arc (`DOME_ARC`), clipped to the dome, in both SVG mascots. |
| SHOULD-FIX 4 | `08b17ca` | Phone hero eyebrow: lighter blue with a dark text halo; darker, taller bottom scrim. |
| SHOULD-FIX 5 | `510075b` | Phone sources copy on a 70 % dark backing; underwater eye glow dimmer and smaller. |
| SHOULD-FIX 6 | `493af14`, reverted by `1486c7b` | Withdrawn by the reviewer: `package-lock.json` was already stale before this work, so regenerating it pulled about 1,500 unrelated lines into a landing-page PR. It is handled in its own backlog item; CI and the Dockerfile install from `bun.lock`. `package-lock.json`, `package.json` and `bun.lock` are byte-identical to `96e3c93`. |
| (my screenshots) | `53b0f24` | Depth stack anchor 0.36 to 0.42 and footprint capped at 0.86 on wide screens, so Silver and Gold no longer run under the last lines of the copy. |

**Verification** on the fixed code (`53b0f24`; the revert after it touches only `package-lock.json`, which bun does not read), foreground: `bun run typecheck` exit 0; `bun run lint` 0 errors, 6 warnings (same six, none in files this work touches); `bun run test` 954 pass, 1 skip, 0 fail, 2353 expect() calls, 955 tests across 105 files; `bun run build` exit 0, `/landing` static. Phone re-check (`checks.js phone`): no console errors, no horizontal overflow, layout shift 0.

**Screenshots** (same folder, prefix `r1-`, taken with `scroll-behavior: auto`): `r1-d-sources-*` and `r1-m-sources-*` (pills above the caption, phone copy on its backing), `r1-d-depthsend-5.5vh/6.4vh` and `r1-m-depthsend-*` (canvas fades, no seam into S4), `r1-m-hero-0` (eyebrow), `r1-d-nowebgl-0` and `r1-m-nowebgl-0` (rim), `r1b-d-final-0.96` and `r1-m-final-1` (final section). Not re-shot after the last commit: nothing but the depth-end frames, which were taken on it.

## 9. Review (planner)

### Round 1 (reviewed at `96e3c93`)

**Re-run by the reviewer, foreground:** `bun run typecheck` exit 0;
`bun run lint` 0 errors, 6 warnings (all in files this work did not touch);
`bun run test` 954 pass, 1 skip, 0 fail (955 tests, 105 files); `bun run
build` exit 0 with `/landing` static. Production server (`bun run start`):
24 screenshots at 1440×900 and 390×844, no page errors, horizontal overflow
0 at both widths. Fallback screenshots (no WebGL, reduced motion) checked.

**Checked and correct:** `/landing` is public with no console chrome
(`PUBLIC_PREFIXES` plus three tests, including the lookalike `/landingpage`);
no API calls, no `@/services` import, no `dangerouslySetInnerHTML`, only the
outside links plan §2.7 allows; `three` is the only new runtime dependency;
CI and the Dockerfile install from `bun.lock`; illustrations are labelled.
The hero, the dive, the three slabs, the chat, the rail, the masking table
and the fallbacks match the plan and look the part.

A note on screenshots: frames taken with `window.scrollTo` far jumps can come
out black, because the page sets `scroll-behavior: smooth` and the capture
lands mid-glide. Wheel scrolling renders every frame. Not a bug.

**BLOCKER 1 — the copy claims more than the product does (principle 2).**
Four of these lines come from the plan itself; the plan was wrong and is
corrected below. Change `src/features/landing/landing-copy.ts`:
1. Sources list: remove **"SFTP"** (the adapter exists but is *Not verified*
   end to end, `PRODUCT.md` §2, `VER-1`) and **"S3-compatible storage"**
   (connectors made in the wizard cannot be tested or have their keys
   changed, `SRC-6`). Nine pills remain.
2. Governance sub: replace "…and the lineage shows where every number came
   from." with "…and lineage shows which tables each table is built from."
   (lineage is Partial and not linked from dashboards, `BI-14`).
3. Governance fact "Lineage from raw table to dashboard" → "Lineage from raw
   table to curated table".
4. Governance fact "Every assistant action logged and approvable" →
   "Assistant actions logged; risky ones wait for approval" (only deletions
   and pauses need approval today, `DEC-8`).
5. Update any screen-reader caption that repeats these lines.

**SHOULD-FIX**
1. **Sources caption overlap.** The falling pills settle on top of "Live
   change capture…" (desktop and phone). Reserve the grid's settled height so
   the caption sits below it at every scroll position.
2. **Hard edge under the Gold slab.** Where S3 ends, the fixed canvas is cut
   by a hard horizontal line across the Gold slab. Fade the canvas out over
   the last part of S3 instead of clipping it.
3. **Stray rim arc on the SVG mascot.** `mascot-svg.tsx` draws a light-blue
   rim stroke that is offset from the dome's edge on the right (visible in
   the no-WebGL hero, the final section and on phones). Align it with the
   dome path or drop it.
4. **Phone hero eyebrow is illegible.** "RANTAI LAKEHOUSE" sits on the bright
   moon reflection at 390 px. Move it above the reflection or give it a
   readable backing (text shadow or a darker scrim under the copy).
5. **Phone sources heading over the glow.** At 390 px the sources headline
   and sub sit on the bright glow of the mascot's eye seen from below; lower
   the glow or add a scrim so body text stays at 4.5:1.
6. **Stale `package-lock.json`.** It does not list `three` or `@types/three`.
   Regenerate it so both lockfiles agree (CI and Docker use `bun.lock`, so
   this is consistency, not a build break).

**Accepted deviations:** the longer section heights, the own scroll-progress
hook, the extra illustration copy (all labelled), the lake-blue approval
card, the phone top bar showing only the wordmark and the button, the
surface hop and no front fill light. The duplicate `theme-color` meta is
harmless; leave the root layout alone.

**Plan corrected** in §4 S2 and S6 to match BLOCKER 1.

### Round 2 (reviewed at `c3e9cbc`)

**Re-run by the reviewer, foreground:** `package-lock.json`, `package.json`
and `bun.lock` byte-identical to `96e3c93` (SHOULD-FIX 6 withdrawn and
reverted in `1486c7b`); `bun run typecheck` exit 0; `bun run lint` 0 errors,
6 warnings (unchanged, none in this work); `bun run test` 954 pass, 1 skip,
0 fail (955 tests, 105 files); `bun run build` exit 0, `/landing` static.
Production server: 18 targeted screenshots at 1440×900 and 390×844 plus six
phone frames through the depth stack with 4.5 s settle time; no page errors,
horizontal overflow 0 at both widths.

**Checked and correct:** BLOCKER 1 (no SFTP, no S3-compatible storage; the
lineage and audit lines say only what is built; the remaining mentions are
the comment explaining why). SHOULD-FIX 1 to 5 fixed as asked: pills settle
above the caption at both widths; the canvas fades out over the end of the
depths section with no seam into the assistant; the SVG mascot's rim light
follows the dome edge; the phone eyebrow and the phone sources copy are
legible. The extra depth-stack anchor change keeps the slabs clear of the
desktop copy.

Not a defect: a phone frame of the depth stack taken after 2.2 s came out
empty under software GL (2 to 4 fps); with 4.5 s every frame shows its slab
and copy.

**No open BLOCKER.** Ready for the pull request. Not verified by anyone: a
GPU or a real phone (frame rate, pixel-ratio caps, tilt), Safari and
Firefox. The product owner checks those on their own devices before calling
`WEB-1` accepted.
