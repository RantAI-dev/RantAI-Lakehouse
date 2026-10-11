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
  Kafka, CSV and TSV files, Excel workbooks, Parquet files, S3-compatible
  storage, SFTP. One pill per item, mono text.
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
  see them, and the lineage shows where every number came from."
- An interactive **masking table** (DOM): 5 rows × 4 columns (name, email,
  city, amount). A toggle "View as: Analyst / Admin". As Analyst, the email
  and name cells **scramble** (random glyph cycling for 400 ms) into masked
  values (`a•••@•••.com`); switching to Admin unscrambles them. Starts in
  Admin and flips to Analyst once when the table scrolls into view.
- Three small facts under it, each a mono label + one line: "Masking proven
  end to end in an acceptance gate", "Every assistant action logged and
  approvable", "Lineage from raw table to dashboard".

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

## 9. Review (planner)
