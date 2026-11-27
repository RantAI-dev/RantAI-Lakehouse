# WEB-5: the Lakehouse site (the landing page as its own static site)

Backlog `WEB-5`. Feature page: `docs/core/features/landing-page.md`. Plan:
`docs/superpowers/plans/2026-11-04-web-5-lakehouse-site.md`.

## Problem

The landing page (`WEB-1`, `/landing`) is a route of the console app.

- **It is not deployed anywhere.** An installed console does not serve it (`WEB-2`, owner
  2026-11-02), so today it is reachable only from a dev server.
- **It pays the console's costs.** It is rendered per request (`force-dynamic` in the root
  layout), it makes a settings API call through the proxy, and it ships the console's provider
  stack.
- **It is English only and has almost no SEO:** no canonical URL, sitemap, robots file, Open
  Graph image or structured data.
- **Its copy stops at 2026-10-11.** Much of what has shipped since is not mentioned.

## Targets (owner signed 2026-11-04, "Google 'Good'")

Measured with Lighthouse in mobile emulation (its default Moto G Power profile, slow 4G) on a
production build of the static export, EN and ID home. The median of three runs is recorded in
`docs/plans/WEB-5-RESULT.md` (principle 5).

| Metric | Target | Best competitor reference |
| --- | --- | --- |
| Largest Contentful Paint | ≤ 2.5 s | Google's "Good" threshold |
| Cumulative Layout Shift | ≤ 0.1 | "Good" threshold |
| Total Blocking Time (lab stand-in for INP) | ≤ 200 ms | "Good" threshold |
| Lighthouse SEO | 100 | — |
| Lighthouse Accessibility | ≥ 95 | — |
| JS loaded before the 3D scene (all initial chunks, gzip) | ≤ 200 KB | — |

The 3D scene is not part of the initial JS. It loads after the page is interactive, and only on a
device with fast WebGL2.

A target that cannot be met is reported in the RESULT file with the measured number. It is not
quietly lowered.
