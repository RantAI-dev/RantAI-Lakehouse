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

## Findings that do not change a score

- Accessibility audit `label-content-name-mismatch` (weight 0, so the score
  stays 100) flags the two language links. Their visible text is "EN" or
  "ID", but their accessible names ("English", "Switch to Bahasa
  Indonesia") do not contain it (WCAG 2.5.3, Label in Name). The RantAI
  site's switcher, which this one copies, has the same pattern. A name such
  as "Switch to Bahasa Indonesia (ID)" fixes it. Not changed here, so the
  measured build stays the build that was reviewed.
- SEO: 100 on both pages. Every SEO audit passes (canonical, hreflang, meta
  description, crawlable links, robots).

## Reviewer's run

To be filled by the reviewer on the final commit (D5): same command, three
runs per locale, medians.
