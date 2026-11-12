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
