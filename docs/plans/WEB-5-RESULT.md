# WEB-5 result: the Lakehouse site, measured

Spec: `docs/core/specs/web-5.md`. Plan: `docs/superpowers/plans/2026-11-04-web-5-lakehouse-site.md`
(D5, T9). This page records numbers, as principle 5 requires: a target that
is missed is written with its number and a cause, and is not lowered.

## What was measured

- **Commit:** `5cf9dfa0` (T8, the last code commit). The site is unchanged
  from T7.
- **Build:** `cd site && NEXT_PUBLIC_SITE_URL=https://example.invalid bun run build`,
  then `site/out/` served by `npx serve@14.2.4 -l 3400 out`.
- **Lighthouse:** 13.6.0 via `npx -y lighthouse@13.6.0 <url>
  --form-factor=mobile --screenEmulation.mobile --throttling-method=simulate
  --only-categories=performance,accessibility,seo,best-practices
  --chrome-flags="--headless=new"`. That is the default mobile profile
  (Moto G Power, slow 4G, 4x CPU, simulated). Chrome 148.
- **Runs:** three per locale (`/` and `/id/`), 2026-11-04 15:42–15:44 local.
  The median of each metric is below; every run follows it.
- **Machine:** a shared VM, 8 cores, with a load average of 11.7 to 14.4
  during the runs (other teams' builds). Lighthouse's `benchmarkIndex` was
  1521 to 1612. The variance between runs below comes from that load. These
  are lab numbers from one loaded machine, not field data.
- **The 3D scene was not part of any run.** Headless Chrome here has only
  SwiftShader. `hasFastWebGL` rejects it by design (D5), so the page ran in
  its DOM-hero mode and never fetched the scene chunk. On a phone with a
  GPU the scene loads after the page is usable (`afterPaint`), so it is
  outside LCP. Its effect on TBT after load is *not verified*.

## Against the targets (median of three)

| Metric | Target | EN `/` | ID `/id/` | Met |
| --- | --- | --- | --- | --- |
| Largest Contentful Paint | ≤ 2.5 s | 1.07 s | 1.07 s | Yes |
| Cumulative Layout Shift | ≤ 0.1 | 0 | 0 | Yes |
| Total Blocking Time | ≤ 200 ms | **217 ms** | **367 ms** | **No** |
| Lighthouse SEO | 100 | 100 | 100 | Yes |
| Lighthouse Accessibility | ≥ 95 | 100 | 100 | Yes |
| JS before the 3D scene (all initial chunks, gzip) | ≤ 200 KB | **209.0 KB** | **209.0 KB** | **No** |
| Lighthouse Performance (recorded, no target) | — | 96 | 91 | — |
| Lighthouse Best Practices (recorded, no target) | — | 100 | 100 | — |

The initial JS is from `bun run size` (`site/scripts/check-bundle.mjs`) on
the same build. It counts every script the page's HTML loads and skips the
`noModule` legacy polyfills (39.6 KB), which module-capable browsers never
download. The deferred scene is one chunk of **146.8 KB** gzip, not
counted. The T5 commit measured 208.9 KB. This build's 0.1 KB more
follows T6's change to `sources-section.tsx` (the pill stagger and phone
classes); the copy itself is in the HTML, not in the JS chunks.

## Every run

| Run | Perf | A11y | SEO | BP | LCP | CLS | TBT | FCP | Speed Index | benchmarkIndex |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| EN 1 | 71 | 100 | 100 | 100 | 1.38 s | 0 | 1642 ms | 1.38 s | 4.12 s | 1586.5 |
| EN 2 | 96 | 100 | 100 | 100 | 1.07 s | 0 | 217 ms | 1.07 s | 1.76 s | 1606 |
| EN 3 | 97 | 100 | 100 | 100 | 1.07 s | 0 | 206 ms | 1.07 s | 1.19 s | 1612 |
| ID 1 | 91 | 100 | 100 | 100 | 1.07 s | 0 | 367 ms | 1.07 s | 1.19 s | 1521 |
| ID 2 | 97 | 100 | 100 | 100 | 1.07 s | 0 | 210 ms | 1.07 s | 1.07 s | 1567 |
| ID 3 | 80 | 100 | 100 | 100 | 1.08 s | 0 | 804 ms | 1.08 s | 2.70 s | 1543.5 |

Total transfer: about 417 KB per page, 20 to 21 requests.

## The two misses

**Total Blocking Time: 217 ms (EN) and 367 ms (ID) against 200 ms.** The
quietest run of each locale is 206 and 210 ms. The long tasks are the
framework's hydration, not the page's code:
- in the median EN run, react-dom's chunk (`3wbm…`) took 727 ms of
  scripting, with long tasks of 222, 111 and 69 ms;
- in the median ID run, the Next App Router chunk (`3--x…`) added a 307 ms
  long task.
In Lighthouse's `bootup-time` audit, the only other chunk listed is
motion's (`3_fhy…`, 73 ms in total, 16 ms of it scripting); no chunk of the
page's own sections appears in it. On this loaded VM the result swings from 206 to
1642 ms between runs of the same build, so the median is as much the
machine as the page. A run on a quiet machine, or field data, is needed
before calling the size of the miss settled. The reviewer's run is the
second measurement (D5).

**Initial JS: 209.0 KB against 200 KB.** About 138 KB of it is the
framework: react-dom 71.9 KB, the Next 16 App Router 45.8 KB, the runtime
and smaller chunks. motion's renderer and its `domAnimation` features are
about 45 KB, and the page's own sections about 25 KB. T5 already removed:
- next-intl's client provider and its ICU parser;
- tailwind-merge (no class attribute on any page changed);
- motion's full `motion.*` bundle, by `LazyMotion` with `m.*`;
- motion's `useScroll`;
- `next/link` from the language links;
- the language offer from the first load.
Further cuts are (a) dropping motion's gesture features through its
internal paths, a few KB, or (b) replacing motion-driven animations with
CSS, which is a design-level change under D6. Both are the planner's and
the owner's call. Until then `bun run size` exits 1 and the CI site job is
red on that step.

## Update: review fix R1 (initial JS)

Measured on the R1 commit with `bun run size`: **181.2 KB** initial JS on
`/` and `/id/`, against 200 KB, so it is **met**, and `bun run size` exits 0.
The scene is unchanged (146.8 KB, deferred), and so are the legacy
polyfills (39.6 KB, `noModule`). Two changes did it:
- **Imports from `framer-motion`.** `motion/react` is
  `import * as fm from "framer-motion"`, plus `const motion = fm.motion`, so
  any import from it kept the whole `motion` component, layout and drag
  included. `framer-motion` 14.9.1 is the package `motion/react`
  re-exports, and its plain re-exports tree-shake. The unused `motion`
  dependency was removed.
- **The animation features after hydration.** `domAnimation` (about 17 KB
  in two chunks) is loaded through a `next/dynamic` component; a bare
  `import()` was written into the HTML as an `async` script (measured
  210.1 KB).

The Lighthouse table above is from `5cf9dfa0`, before R1. The reviewer's
run on the R1 commit is the measurement for TBT.

## Update: review fix R5 (server sections), 2026-11-04

Measured on the R5 build, with the same command and server as "What was
measured": Lighthouse 13.6.0, mobile, simulated, three runs per locale.
The VM's load average was 7.5 to 10.9 during the runs.

| Metric | Target | EN median | ID median | Met |
| --- | --- | --- | --- | --- |
| LCP | ≤ 2.5 s | 1.07 s | 1.07 s | Yes |
| CLS | ≤ 0.1 | 0 | 0 | Yes |
| TBT | ≤ 200 ms | **284 ms** | **218 ms** | **No** |
| SEO | 100 | 100 | 100 | Yes |
| Accessibility | ≥ 95 | 100 | 100 | Yes |
| Initial JS (`bun run size`) | ≤ 200 KB | 178.6 KB | 178.6 KB | Yes |
| Performance (no target) | — | 94 | 96 | — |

| Run | TBT | Longest tasks (ms) | react-dom scripting |
| --- | --- | --- | --- |
| EN 1 | 592 ms | App Router chunk 298; react-dom 291, 127; page 129 | 637 ms |
| EN 2 | 284 ms | react-dom 246, 155; page 122 | 786 ms |
| EN 3 | 230 ms | react-dom 254, 83; page 80 | 846 ms |
| ID 1 | 200 ms | react-dom 229, 83; page 141 | 706 ms |
| ID 2 | 255 ms | react-dom 251, 87, 82; page 85 | 753 ms |
| ID 3 | 218 ms | react-dom 245, 82; page 78 | 716 ms |

Benchmark index 1450.5 to 1612.5. LCP 1.07 s in every run, CLS 0, SEO,
accessibility and best practices 100 in every run.

**TBT is still over 200 ms. The remaining long task is react-dom's
hydration, one task of 229 to 291 ms in every run, which alone is more
than 180 ms of blocking time.** R5 moved the sections' text into server
components; seven of the eight section files are server modules now, and
`"use client"` is on the leaves only. That did not shorten the hydration
task. React still hydrates every host element in the tree, including those
rendered by server components. What it saves is running the components,
not walking the DOM.
- The page has 1,088 elements. 420 of them are the two static dashboard
  tiles: the dot map (280 circles) and the calendar heatmap (140 rects).
- The inline RSC payload is 125.6 KB.

A lever left for the planner and owner, with no design change: draw those
two tiles as one `<img>` each (the same SVG as a data URI, or a static
file). That removes about 40 % of the elements React walks. It was not
done here, because R5 asked for the client/server split and for the
remaining long tasks to be recorded. The reviewer's run and this one differ
(EN 225 / 284 ms); both are on the same loaded VM.

## Update: review fix R6 (static tiles as images), 2026-11-04

The dot map and the calendar heatmap are one `<img>` each: SVG files in
`public/tiles/`, generated from `site/src/lib/tile-svg.ts`. A page goes
from 1,088 elements to 670.

**Measured interleaved,** as in "Reviewer's run 2":
- `382c190d` (before R6) served on :3461, and the R6 build on :3460, both
  `npx serve` of `out/`;
- three rounds of pre-EN, R6-EN, pre-ID, R6-ID with the RESULT's
  Lighthouse command;
- VM load average 2.5 to 8.3 during the runs.

| Metric | Target | EN before | EN R6 | ID before | ID R6 |
| --- | --- | --- | --- | --- | --- |
| TBT (median) | ≤ 200 ms | 202.5 ms | **197.8 ms** | 191.5 ms | **140.1 ms** |
| TBT (runs) | — | 202.5 / 219.5 / 193.6 | 182.2 / 242.0 / 197.8 | 165.5 / 191.5 / 192.0 | 195.4 / 100.5 / 140.1 |
| LCP (median) | ≤ 2.5 s | 1.07 s | 1.07 s | 1.07 s | 1.06 s |
| CLS | ≤ 0.1 | 0 | 0 | 0 | 0 |
| SEO / accessibility / best practices | 100 / ≥ 95 / — | 100 / 100 / 100 | 100 / 100 / 100 | 100 / 100 / 100 | 100 / 100 / 100 |
| Performance (median) | — | 97 | 95 | 97 | 99 |

- **EN's median is 197.8 ms**, under the 200 ms target, but by less than
  the spread between runs (182 to 242 ms). It is a pass on this
  measurement, not a margin.
- **ID's median fell by 51 ms.**
- The longest task is still react-dom's hydration or the App Router
  (207 to 260 ms in the R6 runs).
- One R6 EN run had an LCP of 1.35 s; the median is unchanged.

**Pixel check (D6): not pixel-identical, the edges differ.** Both builds
were captured losslessly (PNG over CDP) with each tile scrolled into view,
and compared pixel by pixel in the browser:

| Viewport | Pixels per tile | Differing pixels (map / calendar) | Max channel delta |
| --- | --- | --- | --- |
| 1440×900 @1x | 72,150 | 4,315 / 3,483 | 53 / 49 |
| 1440×900 @2x | 288,600 | 15,213 / 9,976 | 142 / 96 |
| 390×844 @2x | 144,624 | 11,913 / 14,530 | 166 / 126 |

- A 4× zoom with a difference mask shows that only the antialiased rims
  of the dots and cells differ. Fills, colours, positions and the box size
  are the same: 370 × 195.27 px at 1440, 262.19 × 138.38 px at 390.
- The likely cause is that Chrome rasterizes an SVG *image* differently
  from inline SVG when it scales the 360 × 190 viewBox to a fractional
  box.
- The tile and its `<img>` have identical boxes, so there is no layout
  shift.
- The owner judges at QA whether the rims count as a visual change.

## Update: D14 S6 (the story), 2026-11-04

Measured on `85187993` (D14 S1–S5), interleaved against the R7 commit
`b2634f3d`:
- R7 served on :3461, and the D14 build on :3460, both `npx serve` of
  `out/`;
- three rounds of R7-EN, D14-EN, R7-ID, D14-ID with the RESULT's
  Lighthouse command;
- VM load average 4.8 to 8.4.

| Metric | Target | EN R7 | EN D14 | ID R7 | ID D14 |
| --- | --- | --- | --- | --- | --- |
| TBT (median) | ≤ 200 ms | 227.0 ms | **118.8 ms** | 126.4 ms | **119.3 ms** |
| TBT (runs) | — | 3990 / 227 / 107 | 148 / 114 / 119 | 126 / 177 / 111 | 162 / 118 / 119 |
| LCP (median) | ≤ 2.5 s | 1.07 s | 1.07 s | 1.07 s | 1.07 s |
| CLS | ≤ 0.1 | 0 | 0 | 0 | 0 |
| SEO / accessibility / best practices | 100 / ≥ 95 / — | 100 / 100 / 100 | 100 / 100 / 100 | 100 / 100 / 100 | 100 / 100 / 100 |
| Performance (median) | — | 96 | 99 | 98 | 99 |
| Initial JS (`bun run size`) | ≤ 200 KB | 178.5 KB | 181.1 KB | 178.5 KB | 181.1 KB |

- **Every target is met on the D14 build.** No D14 run has a non-passing
  accessibility, SEO or best-practices audit.
- **Read these numbers as "no regression measured", not as a gain.** The
  D14 medians are lower than R7's, but the story adds work: the page-wide
  water, the gauge, and the deferred bubbles and caustics. R7's EN median
  includes one 3,990 ms run, a load spike on the VM; its other two runs are
  107 and 227 ms. The difference is within this machine's run-to-run
  spread.
- The longest task is still react-dom's hydration or the App Router,
  214–260 ms.
- The 3D scene chunk is 147.0 KB, deferred.
- **Not measured here:** the live-lake path, including the longer 220 vh
  hero and the waterline overlay. Lighthouse's Chrome on this VM has only
  SwiftShader, so these runs are the DOM-hero mode, as in every earlier run
  on this page. The test-only WebGL override is development-only and is not
  in the export (checked by `bun run size`).

## Findings that do not change a score

- Accessibility audit `label-content-name-mismatch` (weight 0, so the score
  stays 100) flags the two language links. Their visible text is "EN" or
  "ID", but their accessible names ("English", "Switch to Bahasa
  Indonesia") do not contain it (WCAG 2.5.3, Label in Name). The RantAI
  site's switcher, which this one copies, has the same pattern. **Fixed by
  review fix R2 (`71ae28a4`):** the names now start with the visible label
  ("EN: English", "ID: Switch to Bahasa Indonesia"). Lighthouse 13.6.0
  (accessibility only, mobile) then scores the audit 1 on `/` and `/id/`,
  and accessibility stays 100.
- SEO: 100 on both pages. Every SEO audit passes (canonical, hreflang, meta
  description, crawlable links, robots).

## Reviewer's run

### Reviewer's run 1: `61b5aa26` (after review fixes R1–R4)

- **Build:** a clean `git archive` of `61b5aa26` into a scratch folder, a fresh
  `bun install --frozen-lockfile`, `NEXT_PUBLIC_SITE_URL=https://example.invalid bun run build`,
  and `out/` served by `npx serve@14.2.4`.
- **Lighthouse:** the same command as above (13.6.0, mobile, simulated throttling, headless
  Chrome 148). Three runs per locale, 2026-11-04 around 16:25 local.
- **Load:** the VM's load average was 4.9 to 7.5 during the runs, lower than the developer's
  (11.7 to 14.4). `benchmarkIndex` was 1587 to 1626.
- **Size:** `bun run size` gives 181.2 KB on both pages (OK), with the scene at 146.8 KB in one
  deferred chunk.

| Metric | Target | EN `/` | ID `/id/` | Met |
| --- | --- | --- | --- | --- |
| Largest Contentful Paint | ≤ 2.5 s | 1.07 s | 1.07 s | Yes |
| Cumulative Layout Shift | ≤ 0.1 | 0 | 0 | Yes |
| Total Blocking Time | ≤ 200 ms | **225 ms** | 199 ms | **EN no**, ID yes |
| Lighthouse SEO | 100 | 100 | 100 | Yes |
| Lighthouse Accessibility | ≥ 95 | 100 | 100 | Yes |
| JS before the 3D scene (gzip) | ≤ 200 KB | 181.2 KB | 181.2 KB | Yes |
| Performance (recorded) | — | 96 | 97 | — |

| Run | Perf | A11y | SEO | BP | LCP | CLS | TBT | FCP | Speed Index | benchmarkIndex |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| EN 1 | 95 | 100 | 100 | 100 | 1.07 s | 0 | 252 ms | 1.07 s | 2.40 s | 1619 |
| EN 2 | 96 | 100 | 100 | 100 | 1.08 s | 0 | 210 ms | 1.08 s | 2.37 s | 1625 |
| EN 3 | 96 | 100 | 100 | 100 | 1.07 s | 0 | 225 ms | 1.07 s | 1.07 s | 1606.5 |
| ID 1 | 97 | 100 | 100 | 100 | 1.07 s | 0 | 199 ms | 1.07 s | 2.56 s | 1587 |
| ID 2 | 95 | 100 | 100 | 100 | 1.07 s | 0 | 244 ms | 1.07 s | 2.17 s | 1610.5 |
| ID 3 | 97 | 100 | 100 | 100 | 1.07 s | 0 | 197 ms | 1.07 s | 2.38 s | 1626 |

- `label-content-name-mismatch` passes in every run (R2).
- **TBT** is now within 197–252 ms on every run, against 206–1642 ms before. The EN median is
  still over the target by 25 ms.
- The longest task in every run is hydration in react-dom's chunk (`3wbm…`, 234 ms in EN 3). In
  ID 1 it is the App Router chunk (`3--x…`, 212 ms).
- Every home section file is a client component, while D5 says sections that only render text
  are server components with client leaves. Review 2 finding R5 asks for that split and a
  re-measure.

### Reviewer's run 2: `382c190d` (R5) against `61b5aa26` (before R5), interleaved

- **Why interleaved:** the developer's R5 runs (EN 284 ms, ID 218 ms) were taken at a load of
  7.5 to 10.9. To compare the builds on equal terms, both were served side by side, from clean
  builds, and run in turn: pre EN, R5 EN, pre ID, R5 ID, three rounds.
- **Conditions:** the same Lighthouse command. The load average was 5.1 to 8.1, and
  `benchmarkIndex` 1440 to 1659.

| Build | EN TBT runs | EN median | ID TBT runs | ID median | Longest task (median) |
| --- | --- | --- | --- | --- | --- |
| `61b5aa26` (before R5) | 212, 215, 222 ms | **215 ms** | 209, 275, 208 ms | **209 ms** | 230 ms |
| `382c190d` (R5) | 213, 241, 162 ms | **213 ms** | 251, 197, 208 ms | **208 ms** | 229 ms |

- Every other metric held in all twelve runs: LCP 1.07–1.08 s, CLS 0, accessibility and SEO
  100, performance 94–98.
- Initial JS at `382c190d` is 178.6 KB.
- **Reading:** R5 neither helped nor hurt TBT on equal terms. Both builds sit about 10 ms over
  200 ms. What remains is one hydration task of about 230 ms, which alone is 180 ms of blocking
  time.
- Review 3 finding R6 tries the developer's suggestion: draw the two static tiles as images, to
  cut the elements React hydrates.

### Reviewer's run 3: `1e5b0bf1` (D14 and R8–R11) against `6b1566c9` (R7), interleaved

- **Conditions:**
  - both builds come from a clean `git archive`, are served side by side, and are run in turn: R7 EN, final EN, R7 ID, final ID, for three rounds;
  - the Lighthouse command is the same as in "What was measured";
  - the load average was 4.0 to 6.6.
- **No GPU:** this VM has none, so these runs measure the page without the 3D scene, as every earlier run did. The 220 vh live hero and the waterline are not part of any number here.

| Build | EN TBT runs | EN median | ID TBT runs | ID median |
| --- | --- | --- | --- | --- |
| `6b1566c9` (R7) | 187.0, 95.0, 150.7 ms | **150.7 ms** | 108.8, 103.0, 121.1 ms | **108.8 ms** |
| `1e5b0bf1` (final) | 108.2, 163.6, 165.1 ms | **163.6 ms** | 200.3, 136.6, 131.8 ms | **136.6 ms** |

- **Every other metric held in all twelve runs:**
  - LCP 1.06–1.07 s;
  - CLS 0;
  - accessibility, SEO and best practices 100;
  - performance 97–99.
- **Initial JS:** 181.5 KB at `1e5b0bf1`, against 178.5 KB at R7. The budget is 200 KB.
- **Reading:** every target is met on the final commit.
  - The final build's TBT medians are 13 ms (EN) and 28 ms (ID) above R7. Both differences are smaller than the spread of R7's own runs, which is 95–187 ms.
  - One final ID run reached 200.3 ms. The median is 136.6 ms.
  - So this is no regression that can be measured on this VM. It is no gain either.

### Reviewer's run 4: `89e049e2` (R12) against `390a482f`, interleaved

- **Conditions:**
  - both builds come from a clean `git archive`, are served side by side, and are run in turn, three rounds;
  - the Lighthouse command is the same as before;
  - the load average was 3.8 to 7.1;
  - there is no GPU, so these runs measure the DOM hero only.

| Build | EN TBT runs | EN median | ID TBT runs | ID median |
| --- | --- | --- | --- | --- |
| `390a482f` (before R12) | 111.5, 96.0, 164.6 ms | **111.5 ms** | 87.2, 194.7, 132.8 ms | **132.8 ms** |
| `89e049e2` (R12) | 105.2, 295.5, 99.3 ms | **105.2 ms** | 148.0, 162.8, 155.5 ms | **155.5 ms** |

- **Every other metric held in all twelve runs:**
  - LCP 1.06–1.13 s;
  - CLS 0;
  - accessibility, SEO and best practices 100;
  - performance 93–100.
- **Initial JS:** 181.5 KB.
- **Reading:** every target is met.
  - EN run 2 of R12, at 295.5 ms, was taken at the highest load of the session (7.1). The other two EN runs of R12 are below the earlier build's median.
  - The ID median is 23 ms higher. That is within the spread of the earlier build's own runs, 87–195 ms.

## 2026-11-25 performance pass (T3)

### Method

- **Builds:** each build is made from a copy of `site/` outside the repo (`NEXT_PUBLIC_SITE_URL=https://example.invalid bun run build`), served with `serve@14.2.4` on two ports and measured side by side, so both see the same machine load.
- **Lighthouse:** `lighthouse@13.6.0`, the hub command (mobile, simulated throttling, `--headless=new --no-sandbox`), three runs per page and build, alternating pages and builds. The median of the three is the number. Load average was 1.6 to 3.4. There is no GPU, so these runs measure the DOM hero (the 3D lake is not part of any number).
- **Versions:** "before" is `3025a6409` with Next 16.4.3. "After" is `809796fae` with Next 16.4.6 (the dependency bump, `c6f2fff84`, is part of it).
- **Look check:** headless Chrome screenshots at 1190×800 and 390×844 (top, governance bento, Open tables panel), before and after, are identical. Page height is identical (14,799 px and 16,549 px). Hovering a lineage column moves the highlight the same way in both builds. No console errors.

### Why the English LCP waited

- **LCP element:** the hero `<h1>` on both pages (`div.sticky > div.mx-auto > div.max-w-[660px] > h1.lk-display`). So there is no difference in what is measured.
- **Cause:** `/` is not slower in itself. The English page runs the full first-visit intro (`hero-intro.tsx`), whose inline script creates a WebGL2 context to decide between the 3D and the DOM path. It ran in `<body>` while the parser was still on it, so it sat in front of the first paint. On a cold browser (Lighthouse starts a fresh profile) creating the first WebGL context starts the GPU process. In 2 of 6 baseline English runs the observed FCP was 2.2 s instead of 0.2 s (LCP 2.4 s and 2.1 s), and on a busy VM it was worse (the 4.2 to 4.8 s LCP of 2026-11-24). `/id/` takes the light intro, which never probes, so it was never hit. The h1 itself is never hidden (it is masked, not at opacity 0), and fonts, the language offer and hydration are not in the delay: the breakdown is TTFB 22 ms plus element render delay 2.2 s on the slow run, against 0.2 s on the others.
- **Fix** (`ce10be3d6`): the probe runs one task after the first paint. The page starts on the 3D hold (black sea, the headline masked, the same first frames as the DOM beats) and drops it if the answer is no. Six runs before: two slow. Six runs after: none (observed FCP 0.16 to 0.22 s). `scene-mode.tsx` only takes the "3D path at hydration" shortcut when the probe has already answered yes; otherwise it takes its ordinary after-idle path.

### Before and after (median of 3, mobile)

| Page | Build | Performance (runs) | LCP | TBT (runs) | FCP | CLS | Speed Index |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `/` | before | 86 / 85 / 85, **85** | 1.66 s | 504 / 528 / 468, **504 ms** | 1.66 s | 0 / 0 / 0 | 2.79 s |
| `/` | after | 87 / 83 / 86, **86** | 1.66 s | 451 / 593 / 487, **487 ms** | 1.66 s | 0 / 0 / 0 | 2.91 s |
| `/id/` | before | 92 / 86 / 88, **88** | 1.67 s | 329 / 523 / 461, **461 ms** | 1.67 s | 0 / 0 / 0 | 1.67 s |
| `/id/` | after | 90 / 92 / 92, **92** | 1.66 s | 350 / 319 / 305, **319 ms** | 1.66 s | 0 / 0 / 0 | 1.68 s |

- Accessibility 97, SEO 100 and best practices 100 in all runs (the 97 is the colour-contrast audit, T4).
- Earlier runs of the same baseline, one hour before (unpaired): `/` 66 / 85 / 86 / 86 / 83 / 76 (two with LCP 2.1 to 2.4 s) and `/id/` 93 / 92 / 93 / 90 / 87 / 83.
- On a quieter machine the English LCP gap of 2026-11-24 is gone: LCP equals FCP on both pages (1.66 s, the floor of the simulated network).

### Misses

- **`/` performance 86 (median), target 90: missed.** TBT 487 ms is the cause. Every other metric is in the green. `/id/` reaches 92.
- **Where the English TBT goes** (unthrottled trace, 4× CPU in the simulation): hydration of the whole page in one task of about 70 ms, one full style recalculation of about 80 ms when the intro ends and its classes leave `<html>` (about 2.9 s), the `motion-ready` class (reveals) and the first layout of the 2.7 k-element page. Removing the language offer and using the light intro on `/` (a visible change, not kept) lowered the median to 399 ms (89, 86, 89), so even those two do not reach 90 alone.
- **Tried and dropped:** wrapping the below-the-fold sections in `<Suspense>` (hoping React would hydrate them in separate tasks) did not move TBT (median 594 ms over four English runs against 486 ms for the same build without it), so it was reverted.
- **Not done, would need the owner:** these change what is seen or how it moves.
  - The first-visit intro on `/` (3D hold, mask, 2.4 s of beats) and its end-of-intro class removal.
  - `content-visibility: auto` on the sections below the hero, which would cut the first layout and style cost but can move pinned and measured sections.
  - Lazy hydration of below-the-fold sections on scroll, which can leave a reveal hidden for a moment on a fast jump.
- **Left as it was:** legacy JavaScript (Next's own polyfill module, about 14 KiB, ships in the framework chunk and cannot be dropped from the app), render-blocking CSS (3 files, 31 KB gzip, needed for the first paint) and forced reflows (about 60 ms in total, in hydration effects that read layout once).

### Bundle budget (`bun run size`)

CI on `4feeb8e23` failed the 200 KB budget (the budget stays at 200 KB). The cause was the two pictures added later, not the Next bump (16.4.3 and 16.4.6 both give 209.4 KB).

| | `/` and `/id/` | `/contact/` and `/id/contact/` |
| --- | --- | --- |
| before | 209.3 KB in 12 files, OVER | 165.8 KB, OK |
| after `809796fae` | **193.7 KB** in 12 files, OK | 165.8 KB, OK |

The column-lineage and "one table, many readers" pictures are split into a stateless view (the server renders it as the page's still picture, with the markup unchanged) and a live version (the loop and the pointer) in a `next/dynamic` chunk that `shared/deferred-live.tsx` mounts after the first paint, only while the picture is off screen. It renders the same elements, so nothing moves. Without JavaScript, under reduced motion, and if the chunk fails to load, the still picture stays. The scene chunk (155.9 KB) and Lenis (5.9 KB) are unchanged and not counted.

### Commits

- `c6f2fff84` `WEB-5 chore(deps): site Next.js 16.4.3 to 16.4.6 (dependency review)`
- `ce10be3d6` `WEB-5 perf(site): run the WebGL probe after the first paint, not before it`
- `809796fae` `WEB-5 perf(site): load the lineage and open-tables pictures after the first paint`

## 2026-11-25 SEO, accessibility and policy pass (T4)

Built with `NEXT_PUBLIC_SITE_URL=https://lakehouse.rantai.dev`. The `next` bump of T4 was already done in `c6f2fff84`.

### Audit of the exported HTML

Checked on `/`, `/id/`, `/contact/`, `/id/contact/` and `404.html`.

| Item | State |
| --- | --- |
| `<html lang>` `en` / `id` per locale; the 404 is `en` | present |
| canonical on each page, absolute on `lakehouse.rantai.dev` | present |
| hreflang `en`, `id`, `x-default` on the four real pages (the 404 has none, by design) | present |
| `sitemap.xml`: both pages, each with `xhtml:link` alternates `en`, `id`, `x-default` | present |
| `robots.txt`: allow all, `Sitemap:` and `Host:` | present |
| title and description per page and language (titles 25 to 39, descriptions 125 to 153 characters) | present; length bounds now tested in `seo.test.ts` |
| Open Graph: title, description, url, site name, `og:locale` plus the alternate, `og:image` absolute with type, 1200 x 630 and alt | present |
| Twitter `summary_large_image` with title, description and image | present |
| `referrer` meta `strict-origin-when-cross-origin` | present |
| JSON-LD `SoftwareApplication`: name, category, `operatingSystem`, AGPL-3.0 license, free | present |
| JSON-LD `SoftwareApplication.offers` (price 0; the page says "Free and open source") | **fixed**: added |
| JSON-LD `Organization` (RantAI, `https://rantai.dev`) with `logo` | **fixed**: logo `https://rantai.dev/rant-ai.png`, the one rantai.dev names itself |
| No rating or review anywhere; `FAQPage` only on `/contact/` and `/id/contact/` | present |
| `404.html`: `noindex`, no canonical, no hreflang, no JSON-LD | present |

### Contrast (`color-contrast`, accessibility 97 to 100)

Failing nodes read from the T3 Lighthouse JSONs. Only the text colour was changed, hue kept; all ratios are against the measured background.

| Selector | Old | New | Ratio |
| --- | --- | --- | --- |
| `.lk-ast-placeholder` (composer placeholder), `[data-ghost]` (the console's ghost prompt) | `#E8F4FF` at 45% (`#728092`) | at 50% (`#7C8B9C`) | 3.98 to 4.60 |
| `.lk-ast-mode` (Ask / Build, both states) | white at 45% (`#7C8592`) | at 47% (`#818A96`) | 4.29 to 4.59 |
| `.lk-otr-type` (column types in the open-tables card) | `#E8F4FF` at 45% (`#728192`) | at 50% (`#7C8B9C`) | 4.02 to 4.60 |
| `.lk-otr-a-caption` ("Together" caption) | `#E8F4FF` at 40% (`#637181`) | at 50% (`#778594`) | 3.50 to 4.64 |
| `.lk-open-pre` line numbers (`span.w-11`), 9 nodes | `#E8F4FF` at 30% (`#4D5B6C`, `#51657B` on the hot band) | text replaced by `::before { content: attr(data-n) }`, colour unchanged | 2.52 / 2.48, no text node left to measure |

The line numbers are the one case that could not be nudged: passing at 4.5:1 needs about 52% alpha, and a gutter at half brightness against 80% code would change the block's look. They sit in an `aria-hidden` block and are `select-none`, so generated content keeps the exact look. The effect is that they are no longer in the DOM: nobody can copy or select them (they could not before either) and the audit has nothing to measure. This is a judgement call worth a second look at review.

### Content Security Policy (meta, first in `<head>`)

```
default-src 'self'; script-src 'self' 'sha256-<one per inline script of that page>'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; media-src 'self'; worker-src 'self' blob:; manifest-src 'self'; frame-src 'none'; object-src 'none'; base-uri 'self'; form-action 'none'
```

- **Changed:** `script-src` no longer has `'unsafe-inline'`. `scripts/csp-meta.ts` hashes each executable inline script of each exported page at build time and writes the hashes into that page's own meta (the home page has 9, the 404 has 3; 37 in all). `application/ld+json` is data and needs no hash.
- **Limit:** `style-src` keeps `'unsafe-inline'` because of React `style` attributes (about 400 on the home page) and motion transforms. Inline styles cannot run script.
- **Check:** the export served on :3401, loaded in headless Chrome 152 (`/usr/bin/google-chrome`, CDP, 1280 x 800), scrolled to the bottom in 500 px steps so the deferred chunks, Lenis and the lazy pictures loaded: **0 `securitypolicyviolation` events and 0 console messages on `/`, `/id/`, `/contact/` and `/id/contact/`**, and the page hydrated (`motion-ready` set). A control run that appended an inline script from the page was blocked and logged one violation, so the listener works.
- **Not covered:** headless Chrome here has no GPU, so the WebGL scene chunk (155.9 KB, loaded when WebGL works) did not run. Its code is from `'self'`, so the policy holds, but this was not observed.

### Limits of the host (D22)

GitHub Pages cannot set response headers. A meta tag cannot carry the rest, so these are limits of the host, not passes:
- no HSTS (`Strict-Transport-Security`; Pages enforces HTTPS on the custom domain, but sends no header the site controls);
- no `frame-ancestors` (ignored in a meta tag, and it logs an error there), so framing is not refused;
- no `X-Content-Type-Options: nosniff`.

### Lighthouse (mobile, simulated throttling, `lighthouse@13.6.0`, :3401, three runs alternating `/` and `/id/`)

| Run | `/` perf | `/` a11y | `/` BP | `/` SEO | `/id/` perf | `/id/` a11y | `/id/` BP | `/id/` SEO |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 86 | 100 | 100 | 100 | 89 | 100 | 100 | 100 |
| 2 | 84 | 100 | 100 | 100 | 86 | 100 | 100 | 100 |
| 3 | 85 | 100 | 100 | 100 | 92 | 100 | 100 | 100 |
| median | **85** | **100** | **100** | **100** | **89** | **100** | **100** | **100** |

- SEO, best practices and accessibility reach 100 on both pages; no accessibility audit remains failing.
- The canonical points at `lakehouse.rantai.dev` while Lighthouse loads `localhost`; Lighthouse did not flag it.
- Performance is unchanged work from T3 (TBT 330 to 560 ms, LCP 1.66 s); this pass did not touch it, so the T3 miss on `/` stands.
- JSONs are in `/tmp/claude-1000/web5-t4-lh/`.

### Checks

`bun run typecheck`, `bun run lint`, tests (the claims test reads `docs/`, so it was run in the repo), build and `bun run size` (home 193.7 KB, contact 165.8 KB, budget 200 KB): all pass.

### Commits

- `2b52c39c8` `WEB-5 seo(site)`: the free offer and the Organization logo, plus the length test.
- `518da111e` `WEB-5 sec(site)`: script hashes in the CSP instead of `'unsafe-inline'`.
- `903ae2fa9` `WEB-5 a11y(site)`: the contrast changes above, alone so it can be reverted.

## 2026-11-25 TBT pass (T3b)

Goal: `/` from 86 to 90 or more (mobile performance, median of 3) with no visible change. Result: **reached, `/` 86 to 91, `/id/` 91 to 93.**

### Method

- Builds in copies outside the repo, "before" at `e98c90a73` on :3402, "after" on :3401, each `NEXT_PUBLIC_SITE_URL=https://example.invalid bun run build`; the same Lighthouse command as T3, runs alternating between the two ports.
- **The clock matters.** `data-tod` follows the visitor's local hour (06:00 to 17:59 is day). The VM is on UTC, so every run made between 06:00 and 18:00 UTC measures the day hero, where the LCP element is the "Free & open source" line at 2.9 s (not the headline) and the score is 68 to 71 (LCP 4.7 s). The T3 and T4 numbers were taken at night. All numbers here run Chrome with `TZ=Asia/Jakarta` (19:00 WIB, night). The day-mode LCP is a finding for the owner, see "Not done".
- The trace (unthrottled, one run) before the change: a style recalculation of the whole page (3,674 elements, 50 ms) and a layout of 2,513 objects (45 ms) right after parsing, a restyle of 3,244 elements (32 ms) in hydration, and a restyle of all 4,400 elements (59 ms, one task) when the intro's classes left `<html>`.

### Levers

| # | Lever | Kept | Effect | Commit |
| --- | --- | --- | --- | --- |
| 1 | `content-visibility: auto` on `#governance`, `#get-started`, `#faq` (home page only), with the measured content heights as `contain-intrinsic-block-size: auto` | yes | `/` TBT median 550 to 339 ms (3 pairs, before levers 4 and the hero fix); first layout and restyle skip about 470 elements | `3ba5d2acd` |
| 4 | The intro's rules ended in `*`, `:not(h1)` and `:nth-child()`, so removing two classes from `<html>` restyled the whole page. The five hero-copy children and the backdrop layers now carry a class the rules end on | yes | end-of-intro restyle 4,407 elements / 38 ms to none (trace); `lk-gl-wait` toggling is also free | `3e95b4c97` |
| 2 | Pause decorative loops until seen | not tried | CSS loops off screen did not show in the top tasks of the trace, so the budget went to 1 and 4 | none |
| 3 | Hydration split | not tried | the hydration task is 78 to 93 ms (one `v8.callFunction`), and `3b3-` (framework) 51 to 66 ms; moving static parts out of client trees needs a separate pass | none |

Intrinsic heights are exact (page height equal to the pixel, and `#get-started` lands at the same `scrollY`) at 390, 768 and 1190 px wide in English and Indonesian. At other widths the estimate can be off by a few pixels until the section has been seen once (then `auto` remembers its real height). Not included: the pinned and sticky sections (depths, dashboards, open) and the viewport-height-sized ones (assistant, final, footer), because their height is not a fixed number.

### Before and after (median of 3, mobile, night)

| Page | Build | Performance (runs) | TBT (runs) | Speed Index | LCP |
| --- | --- | --- | --- | --- | --- |
| `/` | before | 88 / 86 / 86, **86** | 414 / 435 / 507, **435 ms** | 2.77 s | 1.66 s |
| `/` | after | 91 / 90 / 94, **91** | 348 / 381 / 258, **348 ms** | 2.79 s | 1.66 s |
| `/id/` | before | 93 / 89 / 91, **91** | 279 / 425 / 362, **362 ms** | 1.66 s | 1.66 s |
| `/id/` | after | 93 / 94 / 92, **93** | 279 / 258 / 321, **279 ms** | 1.66 s | 1.66 s |

Earlier interleaved sets, for the spread (the VM is shared): `/` before 89 / 81 / 83 (TBT 409 / 728 / 550), with lever 1 only 90 / 92 / 91 (348 / 314 / 339); a later set of `/` with levers 1 and 4 and the hero fix 92 / 87 / 87 (314 / 461 / 469) against 82 / 85 / 88 before (593 / 540 / 425). Single runs swing by 40 to 100 ms; the medians favour the new build every time. Accessibility, SEO and best practices were 100 in every run.

### Checks

- `typecheck`, `lint` and `bun test src` (94 tests) pass; `bun run size`: 193.8 KB initial JS (budget 200).
- Zero CSP violations and zero console errors after scrolling to the bottom (`/` at 1190, `/id/` at 390, and both with `lk-gl` preset), over CDP; the inline-script hash step now covers 39 scripts.
- Page heights equal before and after: 14,799 / 14,864 px (EN / ID at 1190) and 16,549 / 16,760 px (390). `/#get-started` (and the "Install it" click) lands at `scrollY` 11,362 at 1190 and 12,555 / 12,683 at 390, in both builds.
- Pixel comparison, reduced motion (settled), top plus 8 scroll positions, EN and ID, 1190x800 and 390x844 (two builds of the same code differ by 0 pixels): layout rectangles are identical (hero copy rects equal to 0.01 px). Remaining differences are raster only: text anti-aliasing at the top of the 1190 page, and in the governance section content snapped one pixel differently (`content-visibility` paints a section in its own coordinates). 390 ID has 46 differing pixels in one frame, all other frames at 390 differ by up to 6,900 pixels in one or two frames of text edges. No element moved.

### Not done (would need the owner, or a visible change)

- `content-visibility` on assistant, open, final and the footer: their heights follow the viewport height and width (final is 825 px at 1190x800 and 900 px at 1440x900), so the page would be a few pixels off until each had been seen.
- **Day-mode LCP:** between 06:00 and 18:00 local time the LCP is the reassurance line (the hero headline's first paint is smaller than that line's), so a daytime visitor's LCP is about 2.9 s real, 4.7 s simulated. Fixed afterwards without touching the beats, see "Day-mode LCP" below.
- Lazy hydration of the sections below the fold, and moving static markup out of client trees (not attempted here).

### Owner QA 2026-11-25: the DOM hero, then a jump to the 3D lake (`5a6a6549e`)

- **Two causes.** (a) Without the first-visit intro (a refresh in the same session, `/id/`) the scene was only requested after load and an idle period, so the DOM hero (creature, sun or moon centred) showed for seconds before the lake replaced it. (b) A race from `ce10be3d6`: the inline probe runs after the first paint, and if hydration subscribed first, `subscribeWebgl` took the slow path even on the intro's 3D path; the 1.5 s hold then timed out into the DOM beats and the lake came in later.
- **Fix.**
  - Race: the inline probe calls `window.__lkGLHook` when it has answered; scene-mode registers it when hydration comes first, requests the scene chunk on "yes", and gives the answer. The slow path stays as the fallback.
  - Memory: the answer is stored in `localStorage` (`lk-gl` = `1` or `0`, in a try). On a later load with `1` (not on the first-visit intro, not under reduced motion, not without JavaScript) a second inline script adds `lk-gl-wait` to `<html>`. It hides only `[data-hero-room]` (the creature) and the sun, moon, reflection, glitter and waterline layers (`.lk-hold`); the sky and sea, headline, copy and buttons are untouched, so the LCP is the same. The probe runs one task after the first paint and, on "yes", requests the scene chunk at once.
  - Ending: the class stays while the lake is ready (the DOM hero is faded out anyway); it is removed if the probe says no (the answer is stored as `0`), if the scene or its chunk fails, or after 4 s, and the layers fade back in with the transitions they already have (0.4 s for the creature, the day/night 0.6 s for the sun and moon, 0.4 s for the waterline). A stale `1` corrects itself on the next load.
- **Checked in headless Chrome (no GPU, so the probe says no):**
  - `lk-gl` unset or `0`: the creature is at full opacity from the first sample; same as before.
  - `lk-gl` = `1`, intro seen, `/` and `/id/`: `lk-gl-wait` is on and the creature at opacity 0 for the first 100 to 200 ms, then the probe says no, the class goes, the creature fades in over 0.4 s, `lk-gl` becomes `0`.
  - With the probe forced to say yes by a fake context (the real renderer then fails): the scene tries, falls to `none`, the class goes and the creature fades in; never stuck hidden.
  - First visit with the intro, with and without `lk-gl`: same as before. Reduced motion with `lk-gl` = `1`: the creature is visible at once.
  - Zero CSP violations (the new inline script is hashed by the build step); Lighthouse LCP 1.66 s in every run of the table above (a fresh profile has no `lk-gl`).
- **Not verified here:** the 3D path itself (no GPU in the VM): the real lake appearing without a DOM hero in between, on the Mac and the iPhone; the 4 s timeout with a slow real scene.

## 2026-11-25 Day-mode LCP

Goal: `/` by day (06:00 to 17:59 local) back to LCP at FCP (about 1.7 s) and performance in line with night, with no visible change. Result: **reached, `/` by day 74 to 91 (median of 6), LCP 4.7 s to 1.66 s.**

### Cause

- The hypothesis "the headline is painted at opacity 0 by day" is **wrong**. In both modes the headline is masked, not transparent, and it is reported as an LCP candidate at the first contentful paint (360 ms, 22,440 px at 412 px wide).
- By day the hero's reassurance line (`ul > li`) has `text-shadow` with a 26 px blur (the pale wash that keeps it readable over the reflections). Chrome counts a text's shadow in its painted area, so the line's area is **28,086 px**, larger than the headline's 22,440. When the intro reveals the line (beat 3, about 2.2 s), it is a new, larger candidate and becomes the LCP (3.1 s observed on a 4x throttled run, 4.7 s simulated). At night the line has no shadow, so its area stays below the headline's and the LCP stays at FCP.
- Evidence (Chrome `largest-contentful-paint` entries, 412x823, 4x CPU, fresh profile per run, 8 runs each): day always `[H1 22,440 at ~0.6 s, LI 28,086 at ~3.2 s]`, night always `[H1 22,440]`. With `text-shadow: none` on the line by day the candidates are only the H1, and with a transparent blurred shadow on the headline they are only the H1 (33,000 px). Lighthouse (trace `LargestContentfulPaint::Candidate`, `lcp-breakdown-insight`): the element is `ul.lk-ii > li.whitespace-nowrap`, render delay 2.74 s.
- It did not show on every run before: Lighthouse stops observing at the end of the load, so when the late candidate arrived after that the run was clean. Interleaved, `/` by day was 91 / 75 / 95 then 68 / 73 / 73 (LCP 4.69 s in four of six).
- `/id/` (lite intro) and the refresh path (no intro) show the line within the first 0.5 s (LCP is the line at about 0.25 to 0.4 s), so their LCP is already at FCP.

### Fix

One rule in `globals.css`, in the day headline rule: `text-shadow: 0 0 64px transparent`. A fully transparent shadow draws nothing, and Chrome counts its blur in the headline's area (33,440 px at 412, more at other widths), so the headline is the largest candidate again. The intro, its beats and its look are untouched. Tried and dropped: a zero-blur shadow (not counted), a larger blur (same area), vertical offsets (no help).

### Before and after (mobile, Lighthouse 13.6.0, `TZ=Etc/GMT-3` for day, `TZ=Etc/GMT+9` for night; builds on :3402 and :3401)

Day `/` (six runs each, three interleaved):

| Build | Performance (runs) | Median | LCP (runs) | TBT median |
| --- | --- | --- | --- | --- |
| before | 91 / 75 / 95 / 68 / 73 / 73 | **74** | 1.66 / 4.69 / 1.66 / 4.69 / 4.70 / 4.70 s | 336 ms |
| after | 92 / 89 / 86 / 92 / 88 / 94 | **90.5** | 1.66 s in all six | 337 ms |

Other cases (three runs each, sequential builds):

| Case | Build | Performance (runs) | Median | LCP |
| --- | --- | --- | --- | --- |
| day `/id/` | before | 93 / 94 / 94 | 94 | 1.66 s |
| day `/id/` | after | 92 / 93 / 89 | 92 | 1.66 s |
| night `/` | before | 94 / 91 / 93 | 93 | 1.66 s |
| night `/` | after | 93 / 84 / 93 | 93 | 1.66 s |

Accessibility, best practices: 100 in every run. The spread in performance is TBT on a shared VM (the CSS rule does not touch it).

### Checks

- No visible change: 48 intro frames (animations paused and set to 0, 300, 700, 1100, 1500 and 2500 ms), day and night, 1190x800 and 390x844, `/` and `/id/`, before against after: **0 differing pixels**.
- Largest-paint element after the fix is the H1 at 320 (see below), 360, 390, 412, 600, 768, 1190 and 1440 px wide, by day, with the first-visit intro.
- Zero CSP violations (`/` and `/id/` by day, `/` at night). `typecheck`, `lint` pass; `bun run size` 193.8 KB (budget 200); `bun test` 93 of 94 in the copy outside the repo (the claims test reads `docs/`; it passes in the repo, 8 of 8).

### Not verified, not done

- At 320 px wide the headline's area (25,344 px) is still below the line's (28,086 px), so the line still becomes the LCP there by day. That is a 2013-size phone; the next step would be a different hero text size or a shorter line, both visible changes.
- The option that was not taken: showing the reassurance line with the headline (beat 3 earlier) or removing its halo by day. Both change what the visitor sees.
