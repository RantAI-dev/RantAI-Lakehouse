# WEB-5: the Lakehouse site (the landing page moves to `site/`)

Backlog `WEB-5` (Feature). Spec with its targets: `docs/core/specs/web-5.md`.
Feature page: `docs/core/features/landing-page.md`.
Base: `origin/main` at `c8541117`. Branch `feat/web-5-landing-refresh`. Worktree `/home/hv/lakehouse-web5`.

**Roles.** Planner and reviewer: Claude Opus (this session).

Developer: a separate **Opus developer agent**. The product owner decided on 2026-11-04 that
UI work which revamps, merges or creates screens is built by Opus (PR #368 records it). The
developer follows every developer rule in `AGENTS.md`: one task per commit, in plan order, and
no change of scope or decisions. If the plan is wrong against the code, the developer stops and
reports instead of improvising.

## 0. Why

The landing page (`WEB-1`, `src/app/landing`, `src/features/landing`) is a route of the console
app, so it runs into four problems:

- **It is not deployed.** `WEB-2` keeps it out of every console image, so a dev server is the
  only place it renders.
- **It costs what the console costs.** The root layout sets `dynamic = "force-dynamic"`
  (`src/app/layout.tsx:28`), and the proxy reads the content sites through an API call on every
  request (`src/proxy.ts:132`). The page also ships the console's provider stack (React Query,
  nuqs, auth, toaster, tooltip, theme).
- **It is English only, and its SEO is a title and a description:** no `metadataBase`,
  canonical URL, hreflang, sitemap, robots file, OG image or JSON-LD.
- **Its copy stops at 2026-10-11**, while much has been released or accepted since.

The owner chose (2026-11-04) to keep the design and refresh the content, and to take from the
RantAI site (`/home/hv/rantai-page`) its EN/ID setup, SEO, performance, code structure and
practices. The owner also chose a separate static site in `site/` over keeping `/landing`
inside the console, because a static page can be hosted anywhere, can reach the targets, and
makes SEO useful.

## 1. Decisions (owner signed on 2026-11-04 unless marked *planner default*)

- **D1. A separate Next.js app in `site/`, built as a static export** (`output: "export"`). It
  lives in this repo as its own package and is not a bun workspace member:
  - its own `site/package.json` and `site/bun.lock`;
  - Next `16.4.3`, React `19.3.1` and `typescript` at the console's versions;
  - `three` `^0.187.0` and `motion` `^14.9.0` move to it;
  - `next-intl` `^4.14.9` (the RantAI site's version) is new.

  `next build` writes `site/out/`, which any static host serves. It has no server, no API calls,
  no cookies and no analytics.
- **D2. Hosting is decided later** (owner). The site URL comes from `NEXT_PUBLIC_SITE_URL` at
  build time.
  - It must be an absolute `https://` URL. A build without it, or with a bad value, fails with a
    clear message, and there is no default (principle 3).
  - `site/README.md` says how to build and serve `out/`.
  - No Vercel config, no domain and no deploy workflow in this PR.
- **D3. Two languages, as the RantAI site does them.**
  - English lives at `/` and Indonesian at `/id`. No browser-language redirect
    (`localeDetection: false`).
  - An English page shows the small "Tersedia dalam Bahasa Indonesia" offer, a copy of
    `rantai-page/src/components/layout/language-suggestion.tsx`, when the browser prefers
    Indonesian and no choice is stored.
  - An EN · ID switch sits in the top bar.
  - Text lives in `site/src/messages/<locale>/<namespace>.json`, with the same keys in both
    languages. Namespaces: `common` (top bar, footer, skip link, language), `metadata`, `home`
    (the page sections).
  - A key-parity test (the RantAI site's `__tests__/i18n/messages.test.ts`) fails on a missing,
    extra or empty key.
  - There is no middleware in a static export. The developer must make `/` (EN) and `/id` (ID)
    work as plain files.
    - Suggested routing: `app/[locale]` with `generateStaticParams`, plus a root `app/page.tsx`
      that renders the English page; alternatively next-intl's documented static-export setup.
    - **Must verify:** `out/index.html` is English, `out/id/index.html` is Indonesian, and
      neither `/en` nor `/en/` exists in `out/`.
    - If this cannot be done without a middleware, stop and report.
  - Brand and product names are not translated (RantAI, RantAI Lakehouse, Bronze/Silver/Gold,
    Apache Iceberg, the connector names).
  - **Indonesian style (from `rantai-page/AGENTS.md`):**
    - professional and concise, natural rather than literal;
    - the reader is "Anda";
    - tech terms Indonesians use in English stay in English;
    - no longer than the English: a heading must not wrap to more lines than the English on
      desktop or phone.
    - The developer drafts it; the owner reviews it in QA.
- **D4. SEO**, shaped like `rantai-page/src/lib/seo.ts`.
  - `metadataBase` and a title template in the root layout. Each page's
    `generateMetadata` uses a `pageMetadata({ locale, path })` helper that returns:
    - title and description from `metadata.json`;
    - `alternates.canonical`;
    - `alternates.languages` `{ en, id, "x-default": en }`;
    - Open Graph (`type`, `url`, `siteName`, `locale` en_US/id_ID, `alternateLocale`);
    - `twitter: { card: "summary_large_image" }`.
  - An `opengraph-image` per locale, 1200×630, rendered at build time. It shows the logo's
    creature, the wordmark and the localized headline, in the page's night palette.
  - `sitemap.ts`: each page once at its English URL, with `alternates.languages`.
  - `robots.ts`: allows everything and names the sitemap.
  - JSON-LD through a `JsonLd` component that escapes `<`:
    - `SoftwareApplication`, with name, description, `applicationCategory`
      "BusinessApplication", `operatingSystem` "Linux", `license` (the AGPL-3.0 URL), url and
      `isAccessibleForFree`;
    - `Organization`, for RantAI.
    - It carries no `aggregateRating`, `review` or `offers`, because there are none and none are
      invented (principle 2).
  - Icons and `site.webmanifest` are copied from the console's `public/`.
  - The site sets no `noindex` anywhere.
- **D5. Performance: the spec's targets, built as follows.**
  - **Static HTML, no console providers.** Sections that only render text are server
    components. Motion and scroll pieces are client leaves.
  - **The 3D scene loads after the page is usable.**
    - It is a dynamic import started after the first paint, on `requestIdleCallback` or after
      `load`, with a timeout fallback.
    - It loads only when `hasFastWebGL()` passes. That check is a port of
      `rantai-page/src/lib/webgl-support.ts`: `failIfMajorPerformanceCaveat`, and it rejects
      swiftshader and llvmpipe. Otherwise the mode is `none`, the existing DOM hero.
  - **`three` imports are named, not `import * as THREE`,** so the scene chunk can tree-shake.
    The scene chunk's gzip size is recorded.
  - **No React state update per scroll event.** Today `assistant-section` (`setChars`) and
    `depths-section` call `setState` on every scroll. These move to refs, CSS variables or
    motion values, and the visible result stays the same.
  - **Content is visible without JavaScript.**
    - `Reveal` content is not hidden in the server HTML. The hidden starting state applies only
      once a `js` class is on `<html>`, set by a tiny inline script before paint. The
      `<noscript>` patch goes away.
    - The hero `h1` is not wrapped in a filter or opacity start that delays LCP.
  - **Fonts.** The landing's own Instrument Serif (`next/font/local`) and Geist (`next/font`),
    with `display: swap`. The console's dashboard fonts are not loaded.
  - **CI measures the initial JS.** A size check (`site/scripts/check-bundle.mjs`) reads the
    build output and fails above the 200 KB gzip budget. The scene chunk is excluded and
    reported separately.
  - **Lighthouse** (mobile, three runs, median) is run once on the final commit by the
    developer and once by the reviewer. Both are written to `docs/plans/WEB-5-RESULT.md`.
    `npx lighthouse` against `npx serve out` is fine. It is not added as a dependency.
- **D6. The design stays: same scenes, sections, palette and motion.** The content refresh
  works inside the existing sections: new pills, new list items and rewritten lines. It adds no
  new section and no new visual. *(owner: "Pertahankan desain, perbarui isi")*
- **D7. Content: claims only what the backlog says is shipped.**
  - The rule from `landing-copy.ts:1-11` stands: no logos, numbers, testimonials, prices or
    "trusted by".
  - The rule's source changes. A claim must trace to a `docs/core/BACKLOG.md` row with Status
    Released, Done or In Acceptance, because PRODUCT §2's BI rows are stale. Each claim's
    message key gets a sibling comment in a `site/src/content/claims.ts` map:
    `{ key: "home.dashboards.items.3", backlog: "BI-19" }`. A test checks that every claim key
    exists in both languages.
  - **Add (backlog row, status):**
    - **Sources:**
      - the eight SaaS connectors: Salesforce, ServiceNow, Google Analytics 4, HubSpot,
        Zendesk, Confluence, Dynamics 365 and SharePoint (`SRC-3`, Released);
      - uploads of JSON, JSON Lines, Avro, and gzip or zip archives, up to 10 files at once, in
        folders (`DATA-17`/`DATA-18`/`DATA-35`, Released);
      - load modes with dedupe and history (SCD2), and table discovery for every source
        (`SRC-10`/`SRC-13`, Released).
    - **Depths (Silver/Gold):**
      - tables that refresh themselves within a target lag (`BLD-2`, In Acceptance);
      - quality checks with anomaly detection (`DATA-14`, Released).
    - **Dashboards:**
      - permissions per dashboard and folder (`BI-19`, In Acceptance);
      - comments with @mentions (`BI-19`);
      - alerts on a chart (`BI-19`);
      - PNG, PDF and Excel export (`BI-19`, `BI-1`);
      - scheduled reports (`RPT-1`, In Acceptance);
      - public links with expiry and password, and embeds (`BI-19` A, `BI-26`, `SEC-12`);
      - results remembered and refreshed (`BI-4`, In Acceptance);
      - uploaded map boundaries (`BI-16` C2, merged in #246/#265; BI-16 is Building, so this
        says "your own map boundaries", not "every chart type");
      - calculated fields, parameters and reusable metrics (`BI-8`/`BI-11`/`BI-2`, In
        Acceptance).
    - **Governance:**
      - catalog search with certified and deprecated marks and governed tags
        (`DATA-11`/`DATA-12`, Released);
      - column-level lineage (`DATA-13`, Released).
    - **Query Studio** (inside an existing section's list): autocomplete, tabs, history
      (`QS-1`/`QS-4`, In Acceptance).
  - **Do not add:**
    - change capture for MySQL or SQL Server (`SRC-4` Planned; `VER-1` Todo);
    - SFTP (`VER-1`);
    - "every chart type" (`BI-16` Building).
    - Keep "Live change capture for PostgreSQL".
  - **WEB-3 is in progress** and takes Lineage out of the console sidebar. The governance copy
    describes lineage as a capability, not as a sidebar page.
- **D8. Calls to action.** There is no console on the site, so "Open the console" (`/login`)
  becomes **"Install it"**, linking to the repository's install section. The developer confirms
  the anchor in `README.md`. "Talk to us" (mailto) and "Read the source" stay, and so do the
  user guide and the other RantAI sites in the footer. *(planner default)*
- **D9. Code structure, from `rantai-page/AGENTS.md`:**
  - `src/app` is routes only;
  - a route's own pieces go in `_components/`, imported relatively;
  - pieces shared by several routes go in `src/components/<role>/` (`layout`, `shared`,
    `effects` for the scene);
  - non-UI code goes in `src/lib`;
  - kebab-case file names, server-first, no hard-coded UI text.
  - The scene engine (`scene/*.ts`) moves as is apart from D5's imports. Its tests
    (`scramble.test.ts`, `scroll-timeline.test.ts`) move with it.
  - Tests use `bun test`, as the console does, with its own `site/bunfig.toml`.
  - The site keeps the console's lint rules (eslint-config-next core-web-vitals + typescript).
  - Never run prettier (repo rule).
- **D10. Accessibility.**
  - Keep the skip link, the real `h1`/`h2`s and the reduced-motion and no-WebGL paths.
  - Add `lang` per locale, and `hrefLang`/`lang` on the switcher links.
  - Add `aria-current` on the active language and `focus-visible` outlines.
- **D11. The console loses the landing page.**
  - Delete `src/app/landing` and `src/features/landing`.
  - Remove `/landing` from `PUBLIC_PREFIXES` (`src/features/auth/auth-provider.tsx:36`) and its
    comment, and update `auth-provider.test.ts` so that `/landing` is not public.
  - Remove `three` and `@types/three` from the root `package.json`; update `bun.lock` with
    `bun install`.
  - `.dockerignore`: the two landing lines become one `site/` line, keeping the `WEB-2` reason
    comment updated.
  - `ops/release/test_release_workflow.py` `LANDING_PATHS` asserts `site` is excluded.
  - `release.yml`'s `/landing`→404 install proof stays. It is still true, and it guards
    against a re-added route.
  - Root `tsconfig.json`, `eslint.config.*` and `bunfig.toml`/test discovery must ignore
    `site/`, so console checks never pick up the site's files.
- **D12. CI.**
  - The `changes` job gains a `site` output for paths `site/**`.
  - A new job `site` (name "Site · lint, typecheck, test, build, size") runs in `site/`, when
    `site` or `all` changed and on every push to main:
    - `bun install --frozen-lockfile`;
    - lint, typecheck and test;
    - `next build` with `NEXT_PUBLIC_SITE_URL=https://example.invalid`;
    - the bundle-size check;
    - a check that `out/index.html` has `lang="en"` and `out/id/index.html` has `lang="id"`.
  - Add the job to `scripts/ci/required_gate.sh` the same way the TypeScript `verify` job is
    required. The developer reads that script first.
- **D13. No assistant change.** This is not a BI feature, so the assistant-parity rule does not
  apply.

## 2. Anchors (at `c8541117`)

- `src/app/landing/page.tsx`: metadata (`:5-16`), viewport (`:19-21`), the noscript patch
  (`:37-39`).
- `src/features/landing/`, 37 files:
  - `landing-page.tsx:23` (the scene `dynamic(..., { ssr: false })`);
  - `landing-copy.ts` (copy, links `:13-26`, footer `:183-204`);
  - `landing-mode.tsx` (modes pending/full/still/none);
  - `scene/lake-engine.ts:56-64` (DPR caps) and `:353-408` (pause);
  - `scene/webgl-support.ts`.
- Hard-coded text to move into messages:
  - `top-bar.tsx:36`;
  - the `landing-footer.tsx` column titles;
  - `depths-section.tsx:35,44,79`;
  - `assistant-section.tsx:24-29`;
  - `dashboard-tiles.tsx:156-161` and the SVG aria-labels;
  - `governance-section.tsx:~66-75`;
  - the skip link in `landing-page.tsx:37`.
- Per-scroll state: `assistant-section.tsx` (`setChars`) and `depths-section.tsx`.
- Console references to remove:
  - `src/features/auth/auth-provider.tsx:21-36` and `auth-provider.test.ts:5-11`;
  - `.dockerignore:55-63`;
  - `ops/release/test_release_workflow.py:321,1475,2359-2413`;
  - `release.yml:1554-1564`, which stays.
- RantAI site references (read only; its working tree has the owner's uncommitted work, so do
  not touch it):
  - `src/i18n/{routing,request,navigation,paths}.ts`;
  - `src/lib/seo.ts:16-53`, `src/lib/og-image.tsx`, `src/components/shared/json-ld.tsx`;
  - `src/components/layout/{language-switcher,language-suggestion}.tsx`;
  - `src/lib/webgl-support.ts`;
  - `__tests__/i18n/messages.test.ts`.

## 3. Tasks (one commit each, in order)

1. **T1 Scaffold `site/`.**
   - What it adds:
     - `site/package.json` (scripts `dev`, `build`, `lint`, `typecheck`, `test`, `size`) and
       `bun.lock`;
     - `next.config.ts` (`output: "export"`, `images.unoptimized`, `trailingSlash: true`, the
       `NEXT_PUBLIC_SITE_URL` check);
     - `tsconfig.json`, `eslint.config.mjs`, `bunfig.toml`;
     - `src/app` with a placeholder page;
     - `site/README.md`;
     - the root ignores (D11's last bullet);
     - the CI job (D12).
   - *Accept:*
     - `cd site && bun run build` writes `out/index.html`;
     - a build without the env var fails with the message;
     - the root `bun run typecheck && bun run lint && bun run test` is unchanged.
2. **T2 i18n.**
   - What it adds:
     - next-intl routing, request config, navigation and paths;
     - `[locale]` layout with `lang`;
     - the language switcher and the suggestion;
     - the messages files;
     - the key-parity test.
   - *Accept:*
     - `out/index.html` is `lang="en"`, `out/id/index.html` is `lang="id"`;
     - no `out/en/`;
     - the parity test passes, and it fails when a key is deleted (show that in the handoff).
3. **T3 Move the page, English copy unchanged.**
   - The sections, scene, mascot, CSS and fonts go into the D9 structure.
   - Every string goes into `messages/en/home.json` and `common.json`, and `id` gets the same
     keys with the English text for now.
   - D8's CTA change.
   - *Accept:* side-by-side screenshots (headless Chrome over CDP, desktop 1440 and phone 390)
     of the console's old `/landing` and the site's `/`, at the hero, depths, dashboards and
     footer. They look the same, apart from the CTA.
4. **T4 SEO.**
   - `pageMetadata`, `metadataBase`, `opengraph-image` per locale, sitemap, robots, JSON-LD,
     icons and manifest.
   - *Accept:*
     - the built HTML of `/` and `/id/` has canonical, three hreflang links, og:url and
       og:image;
     - `out/sitemap.xml` and `out/robots.txt` exist;
     - the JSON-LD parses;
     - a unit test covers `pageMetadata` and the paths helper.
5. **T5 Performance (D5).**
   - Deferred scene with `hasFastWebGL`, named `three` imports, no per-scroll `setState`,
     visible-without-JS reveal, fonts, the size check.
   - *Accept:*
     - `bun run size` passes and prints the initial and scene sizes;
     - the reduced-motion, no-WebGL and no-JS runs still show every section (CDP screenshots).
6. **T6 English content refresh (D7).**
   - The new claims, with the `claims.ts` map and its test.
   - *Accept:* every new line traces to a backlog row in `claims.ts`; nothing from the "do not
     add" list appears.
7. **T7 Indonesian (D3 style).**
   - The full `id` copy, plus `metadata` and the OG text.
   - *Accept:*
     - parity test green;
     - CDP screenshots of `/id/` at 1440 and 390: no heading wraps to more lines than its
       English, or the handoff lists each one.
8. **T8 Remove the landing from the console (D11) and update the docs.**
   - `landing-page.md`: where it lives and the new routes; the checklist targets the site.
   - `WEB-1`/`WEB-2` mentions in `BACKLOG.md` stay as history.
   - *Accept:*
     - the root checks are green;
     - `grep -rn "features/landing\|app/landing" src` is empty;
     - `bun.lock` has no `three`.
9. **T9 Measure (D5) into `docs/plans/WEB-5-RESULT.md`.**
   - It records: Lighthouse mobile EN and ID (three runs, median: LCP, CLS, TBT, SEO, a11y,
     performance score), the initial JS and the scene chunk (gzip), and the commit measured.
   - A miss is written with its number and a cause (spec: never lowered quietly).

## 4. PR slicing

One PR: a draft from the first push, so CI runs on every push. Title
`WEB-5 feat(site): the Lakehouse landing page as its own static site, in English and Indonesian`.
The planner opens it.

## 5. Out of scope

- Hosting, a domain, a deploy workflow and analytics (D2).
- Any design change: no new section, scene or palette (D6).
- A link to the site from inside the console (`WEB-1` open question 1 stays open).
- The RantAI site's CMS, blog, Drizzle, Vercel analytics and Lenis.
- Changing `rantai-page` in any way.

## 6. Must verify, not assume

- The static-export i18n routing (D3).
- That `opengraph-image` renders under `output: "export"`. If it does not, generate the two PNGs
  at build time with a script and reference them statically. Say which one was used.
- The README install anchor (D8).
- That `required_gate.sh` really requires the new job on a PR that touches only `site/`.

## 7. Handoff (developer appends)

### Handoff 1: developer (Opus), 2026-11-04, one PR slice (#379)

**Commits** (`96cf5c29..a1cce859` on `feat/web-5-landing-refresh`, all pushed):

| Commit | What |
| --- | --- |
| `4eeaeb86` | T1 scaffold `site/`: static export, the `NEXT_PUBLIC_SITE_URL` check, the CI `site` scope, job and gate, root ignores |
| `2933129e` | T2 i18n: `/` English and `/id/` Indonesian with no middleware, the switcher and offer, the parity test |
| `14d0865f` | T3 the page moved into the D9 structure, every string in messages, "Install it" |
| `8253865e` | T4 SEO: metadata, OG cards, sitemap, robots, JSON-LD, icons and manifest |
| `4478b8e4` | Review fix B1: `site/bun.lock` regenerated by bun 1.3.12 (lockfileVersion 1) |
| `ba815ffa` | Review fix B2: the root `bun test` ignores `site/` (`pathIgnorePatterns`) |
| `e64285ec` | T5 performance: deferred scene, named `three`, no per-scroll state, readable without JS, the size check |
| `9c548d63` | T6 English content refresh, `content/claims.ts` and its test |
| `2fb58f1e` | T7 Indonesian copy, metadata and OG |
| `5cf9dfa0` | T8 the console loses the landing page; the feature page targets the site |
| `a1cce859` | T9 `docs/plans/WEB-5-RESULT.md` |

**Final verification**, in the foreground on the final code (`a1cce859`):
- `cd site && bun run lint && bun run typecheck && bun run test && bun run build && bun run size`:
  - lint, typecheck and build exit 0;
  - test: 45 pass, 0 fail;
  - **size exits 1**: 209.0 KB initial on `/` and `/id/` against 200 KB. The scene is 146.8 KB, deferred; the `noModule` polyfills (39.6 KB) are listed and not counted.
- Root `bun run typecheck`: 0. `bun run lint`: 0, with the 10 warnings `main` already has.
- Root `bun run test` under bun 1.3.12 (CI's version): 3204 pass, 1 skip, 0 fail of 3205, 326 files, no `site/` path in the output.
  - For this local run only, `src/features/admin/users-page.test.tsx` was added to the ignore list. It hangs on this VM on the base commit too (checked), and with it the whole run is SIGKILLed; CI runs it.
  - B2's own proof was a run with `site/node_modules` moved aside: 3223 pass, 1 skip, 0 fail (its commit body).
- `python3 ops/release/test_release_workflow.py`: 118 tests OK. pytest is not installed on this VM; CI runs the file with pytest.
- `bash scripts/ci/tests/test_detect_change_scope.sh`: 40 passed. `bash scripts/ci/tests/test_required_gate.sh`: 62 passed (both at T1; untouched since).
- `docker compose ... config`: not run, because compose was not touched.
- `git status`: clean.
- CI: the run on `ba815ffa`, after the two blocker fixes, has Site and Frontend green. Every later run was cancelled by the next push (`cancel-in-progress`). The run on `a1cce859` will show the site job red on the new "Check the bundle size" step, which is the 209.0 KB miss.

**Against the spec** (`docs/plans/WEB-5-RESULT.md`, Lighthouse 13.6.0 mobile, three runs, medians EN / ID):

| Target | Result |
| --- | --- |
| LCP ≤ 2.5 s | 1.07 s / 1.07 s: met |
| CLS ≤ 0.1 | 0 / 0: met |
| TBT ≤ 200 ms | **217 / 367 ms: missed** (runs 206–1642 ms on a VM at load 11.7–14.4; react-dom hydration and App Router long tasks) |
| SEO 100 | 100 / 100: met |
| Accessibility ≥ 95 | 100 / 100: met |
| Initial JS ≤ 200 KB | **209.0 KB: missed** (framework ~138 KB, motion ~45 KB, sections ~25 KB) |

**Bundle sizes** (gzip):
- Initial JS on both pages: 209.0 KB. T5's reduction steps are in its commit body:
  - 273.6 KB counted with the polyfills; 234 KB without them;
  - 222.1 KB after dropping next-intl's client provider;
  - 221.9 KB with LazyMotion and `m.*`;
  - 213.7 KB without tailwind-merge;
  - 208.9 KB after replacing `useScroll`, deferring the offer and using plain language links.
- The 3D scene: 146.8 KB, deferred.
- Legacy polyfills: 39.6 KB, `noModule`, not counted.

**Misses, for the planner and owner** (reported, not lowered):
1. The initial JS is 209.0 KB, so the CI site job is red on its size step.
   - The remaining options are cutting motion's gesture features through its internal paths (a few KB), or replacing motion-driven animations with CSS (a D6-level change).
   - Neither was done: the first relies on motion internals, and the second changes the design.
2. TBT is 217 / 367 ms. It is dominated by framework hydration, and this VM's load makes the median unstable. The reviewer's run is the second measurement.

**Choices between options the plan allowed:**
- **Routing (D3).**
  - The top `app/layout.tsx` renders no `<html>` (next-intl's documented shape for a root not-found without middleware).
  - `app/(en)/` is a route group for English at the root.
  - `app/[locale]/` writes only the non-default locales (`generateStaticParams` + `dynamicParams = false`).
  - So `out/index.html` is `lang="en"`, `out/id/index.html` is `lang="id"`, and there is no `out/en/`. Verified in T2, and CI checks it.
- **Two departures from the RantAI site, found while verifying T2:**
  - next-intl's `Link locale=` points English at `/en/` for a middleware to redirect, so the language links are plain hrefs from `localizedPath`;
  - next-intl's `createNavigation` bundles the code that writes the `NEXT_LOCALE` cookie, so `i18n/navigation.ts` is a prefix-stripping `usePathname` (D1: no cookies).
  - T5 also dropped `NextIntlClientProvider`: all text reaches the client as props.
- **OG images (§6), the fallback.**
  - The `opengraph-image` convention writes extensionless files under `output: "export"`.
  - `generateImageMetadata` with a `.png` id fails to build for `[locale]` ("returned incomplete params").
  - So the two PNGs are force-static route handlers (`app/(en)/og.png`, `app/[locale]/og.png`) rendered inside `next build`, and `pageMetadata` references them by path.
- **The home page's place (D9).** It is shared by two routes, so it lives in `components/home/`, not in a route's `_components/`.
- **Geist and Geist Mono (D5).** Copied from `design-system/fonts/` and loaded through `next/font/local`, as the console does (no network at build).
- **Console look parity (D6).** `globals.css` restates the console design system's radius scale and base type rules that the landing inherited, so T3's screenshots match.
- **The webmanifest.** Copied from the console, with the name "RantAI Lakehouse" (the console's says "Rantai Lake") and the abyss as background.
- **Query Studio (D7, "inside an existing section's list").** No list on the page fits it, so it is a closing sentence of the assistant section's lead. Please confirm the placement.
- **Sources.** 19 pills instead of 9. The droplet stagger scales so the last still lands in the pin, and on phones the pills and gaps are tighter so 390×844 holds the whole stage.
- **The size budget.** 200 KB is read as 200,000 bytes (the stricter reading). Without the `noModule` polyfill bundle, which module browsers never fetch, the count was 234 KB rather than 273.6 KB.
- **Dependencies beyond D1's list,** needed for the page to move as it is:
  - `clsx` and `lucide-react`;
  - Tailwind v4 (`tailwindcss`, `@tailwindcss/postcss`) and `eslint-config-next`;
  - tailwind-merge, added in T1 and removed in T5.

**Not verified, and why:**
- The live 3D lake. Headless Chrome on this VM has only SwiftShader, which `hasFastWebGL` rejects by design. Every screenshot and Lighthouse run is the DOM-hero mode, and the scene's after-load cost on a GPU phone is not measured.
- The language offer appearing for an Indonesian browser: left to `WEB-5-AC10`, it was not driven here.
- The Indonesian wording is drafted by the developer; the owner reviews it in QA (D3).

**Known limits:**
- At 360×740 the sources stage clips the top of its headline under the top bar (390×844, the spec viewport, fits).
- Without JavaScript, the dashboard tiles' line and Sankey strokes stay undrawn (motion's `pathLength` start).
- Lighthouse's weight-0 `label-content-name-mismatch` on the EN/ID links is recorded in the RESULT with its fix, not applied.

**Process notes:**
- The VM's bun is 1.4.0. From B1 on, every site and root lockfile change was made with a bun 1.3.12 binary (`@oven/bun-linux-x64@1.3.12`, in the scratchpad).
- `next dev` on the console (the T3 comparison) appends an agent-rules block to `AGENTS.md`. It was restored with `git checkout` and never committed.
- Evidence screenshots: `docs/plans/web-5-screens/`, 7 files (T3 before/after ×2, T5 three modes, T7 `/id/` ×2).

### Handoff 2: developer (Opus), 2026-11-04, review 1 fixes

**Commits** (`d4c8ba2e..d05fde85`, all pushed; one per finding, R2 plus a follow-up):

| Commit | What |
| --- | --- |
| `a5c6fdd0` | R1 BLOCKER: initial JS 181.2 KB (was 209.0 KB) |
| `71ae28a4` | R2: label in name on the EN/ID links |
| `3760073c` | R2 follow-up: the RESULT note (`71ae28a4`'s body claimed it, but the edit had not applied) |
| `81822b80` | R3: `codeRepository` on a `SoftwareSourceCode` node |
| `d05fde85` | R4: the sources stage fits 360×740 |

**R1: what was measured and changed.**
- The review's form, motion's async `LazyMotion` with
  `features={() => import("./dom-animation")…}`, measured **210.1 KB**.
  Turbopack writes a bare `import()` target into the HTML as an `async`
  script, so the chunk stays in the first load.
- **The cause:** `motion/react` is
  `import * as fm from "framer-motion"; export *…; const motion = fm.motion`.
  Any import from it keeps the full `motion` component (layout and drag),
  so neither `LazyMotion` nor `m.*` could shed code (T5's step saved
  0.2 KB).
- **Fix 1, for review:** all 15 site files import from `framer-motion`,
  added at **14.9.1**, the exact version `motion` 14.9.1 had installed.
  It is the package `motion/react` re-exports, and its plain re-exports
  tree-shake. The unused `motion` dependency was removed. Both changes
  were made with bun 1.3.12 (lockfileVersion 1). This replaces D1's
  "motion ^14.9.0" with its React package.
- **Fix 2:** `domAnimation` arrives after hydration through a
  `next/dynamic` component (`DomAnimationLoader`, `ssr: false`), which is
  not written into the HTML, feeding an async `LazyMotion`. The feature
  code is in two deferred chunks, about 17 KB.
- **Result:** 181.2 KB on `/` and `/id/`, so `bun run size` exits 0. The
  scene is 146.8 KB, deferred; the `noModule` polyfills are 39.6 KB,
  listed and not counted.

**R1: the review's conditions.**
- The hero `h1` and the CTAs use no reveal and no feature, so LCP never
  waits on the chunk.
- **A failed features chunk must not blank the page.**
  - Reveals are hidden only once `motion-ready` is on `<html>`, which is
    set when the features load. This replaces T5's `js` class and its
    inline script.
  - Blocking the deferred chunks over CDP (`Network.setBlockedURLs`) first
    showed that a failed `next/dynamic` chunk replaces the whole page with
    Next's "This page couldn't load".
  - So `QuietBoundary` now wraps all three deferred pieces: the features
    loader, the 3D scene (its boundary calls `markFailed`, so the DOM hero
    stays) and the language offer.
  - Re-measured blocked: the page intact, `motion-ready` false, every
    in-view reveal at opacity 1. Loaded: `motion-ready` true, and reveals
    animate.
- **A regression found while retaking the screenshots, and fixed.** Before
  the features arrive, an `m` element keeps its first rendered style.
  Under reduced motion the sources headline and the chat window stayed at
  opacity 0. Each `m` element whose style switches on `reduced` (and the
  hero copy, on `live`) is now keyed on that state. Probed afterwards:
  opacity 1.
- `docs/plans/web-5-screens/t5-{reduced,nowebgl,nojs}.jpg` were re-taken;
  their content matches T5's.

**R2.** The names are now "EN: English" and "ID: Switch to Bahasa
Indonesia" on `/`, and "EN: Ganti ke English" and "ID: Bahasa Indonesia"
on `/id/`. Lighthouse 13.6.0, accessibility only, scores
`label-content-name-mismatch` 1 on both pages; accessibility stays 100.

**R3.** The SoftwareApplication has an `@id` and no `codeRepository`. A
SoftwareSourceCode node carries `codeRepository`, the license,
`targetProduct` pointing at that `@id`, and the publisher. The test was
updated (46 tests). The built JSON-LD of both pages parses.

**R4.** Phone-only spacing:
- the stage gets `pt-16`, so it centres below the bar;
- the headline box `p-3`, and the lead `mt-3` with line height 1.45;
- the pills `mt-5` and the caption `mt-4`.

Measured (headline-box top / caption bottom; the bar ends at 65 px):

| Viewport | EN | ID |
| --- | --- | --- |
| 360×740 | 91 / 713 | 111 / 693 |
| 390×844 | 196 / 712 | 163 / 745 |
| 1440×900 | 197 / 703 | 197 / 703 (desktop unchanged) |

Evidence: `docs/plans/web-5-screens/r4-sources-360x740.jpg` (EN and ID).

**Checks.**
- Per commit, the scoped site block (lint, typecheck, test, build, size).
- Final, on `d05fde85`'s code, the site block: lint 0, typecheck 0,
  `bun run test` 46 pass, 0 fail, build 0, `bun run size` 0
  (181.2 KB / 181.2 KB).
- No root, compose or release files changed in R1–R4, so the root checks
  were not re-run. Their last run is in Handoff 1 (at `a1cce859`).
- `git status`: clean.

**Not done, as asked.**
- The RESULT's "Reviewer's run" section is untouched.
- A dated "Update: review fix R1" section records 181.2 KB; the
  Lighthouse table stays as measured at `5cf9dfa0`.
- I did not re-run Lighthouse for TBT: the reviewer's run on this commit
  is that measurement.

**For the planner.** R1 swaps D1's `motion` dependency for
`framer-motion` 14.9.1 (same code, the package `motion/react` re-exports).
Please confirm it or ask for it back. Keeping `motion` means importing
through its namespace re-export, which brings back the full bundle.

### Handoff 3: developer (Opus), 2026-11-04, review 2 (R5)

**Commit:** `134cf568` WEB-5 review fix R5 (base `9c5b5148`), pushed.

**What changed.**
- All eight `components/home/*-section.tsx` files are server modules, with
  `"use client"` only on the leaves:
  - `hero-motion` (copy fade, cue fade, backdrop fade);
  - `sources-motion` (headline fade, falling pills);
  - `depths-stage` (pinned stage);
  - `assistant-chat`;
  - `dashboards-scroller` (section height, sliding rail);
  - `governance-table` (table, toggle, flip);
  - `final-mascot` (creature, eye).
- Other pieces:
  - the shared `ProgressSection` measures a section's scroll progress and
    provides it to the leaves;
  - `mascot-svg` is server-only, and the moving creature is in
    `mascot-rising`;
  - the dot map and calendar tiles are server SVG
    (`dashboard-tiles-static`).
- The text is still server props from `getMessages`; there is no client
  provider.

**No visual change (D6).**
- **Server HTML,** R5 against the pre-R5 build, every start tag with its
  attributes plus every text node: `/` has 1082 tags and 205 text nodes,
  `/id/` 1068 and 198, on both builds. The only differences are React's
  `useId` prefix (now a server id) and the static logo's eye `<g>`, which
  lost motion's inert `transform:none` style (two per page).
- **The T5 set** (`docs/plans/web-5-screens/t5-*.jpg`) was re-taken and
  shows the same content.
- **Mid-pin states** were compared with T8 (`5cf9dfa0`), built in a
  scratch worktree that has since been removed: sources, depths,
  assistant, dashboards and the final call match.

**A regression from R1, found and fixed in this commit.** The dashboards
rail no longer slid. With the animation features loading after hydration,
a ref on an `m.*` element was not set in time, so the track was never
measured.
- At T8: the section is `calc(972px + 100vh)` and the rail is at −292,
  −680 and −972 px at 0.3, 0.7 and 1.
- On `d05fde85`: `calc(0px + 100vh)` and no transform.
- The refs now sit on plain elements, and the probe gives T8's numbers
  again. No other ref is on an `m.*` element.

**Commands and counts.**
- Site block on the final code: lint 0, typecheck 0, `bun run test`
  46 pass, 0 fail, build 0, `bun run size` 0. Initial JS is 178.6 KB on
  both pages (was 181.2 KB); the scene is 146.8 KB, deferred.
- Lighthouse 13.6.0, the RESULT's command, three runs per locale. The
  medians are in `WEB-5-RESULT.md` under "Update: review fix R5";
  "Reviewer's run 1" is untouched.

| Metric | EN | ID |
| --- | --- | --- |
| LCP | 1.07 s | 1.07 s |
| CLS | 0 | 0 |
| SEO / accessibility / best practices | 100 | 100 |
| Performance | 94 | 96 |
| **TBT** | **284 ms** | **218 ms** |

**TBT is still over 200 ms.**
- The remaining long task is react-dom's hydration: 229–291 ms in every
  run, which alone is more than 180 ms of blocking time. EN run 1 also had
  a 298 ms App Router task, at TBT 592 ms on a VM at load 7.5–10.9.
- R5 did not shorten hydration. React hydrates every host element, whether
  a server or a client component rendered it.
- 420 of the page's 1,088 elements are the two static tiles' SVG shapes
  (280 circles and 140 rects), and the inline RSC payload is 125.6 KB.
- **A lever left for you and the owner:** draw those two tiles as one
  `<img>` each, the same SVG as a data URI or a static file, with no
  design change. It is not done here; R5 asked to record.

### Handoff 4: developer (Opus), 2026-11-04, review 3 (R6)

**Commit:** `66606e38` WEB-5 review fix R6 (base `2e5468b6`), pushed.

**What changed.**
- The dot map and the calendar heatmap are one `<img>` each:
  `public/tiles/dot-map.svg` and `calendar-heatmap.svg`, with the tile's
  aria text as `alt` and an explicit 360×190 size. A plain `<img>`, because
  `next/image` is a client component.
- `src/lib/tile-svg.ts` is the single drawing (the same shapes and numbers
  as before), and `bun run tiles` writes the files.
- `tile-svg.test.ts` fails if the committed files drift from the
  generator. It also checks 280 circles, 140 rects and the viewBox.
- A page has 670 elements, down from 1,088.

**Measured, interleaved** against `382c190d`:
- `npx serve` on :3461 (before) and :3460 (R6); :3400 was not touched;
- three rounds of pre-EN, R6-EN, pre-ID, R6-ID with the RESULT's
  Lighthouse command, VM load 2.5–8.3.

| Metric | Before | R6 |
| --- | --- | --- |
| EN TBT median | 202.5 ms | **197.8 ms** (runs 182 / 242 / 198) |
| ID TBT median | 191.5 ms | **140.1 ms** (runs 195 / 101 / 140) |

- LCP is about 1.07 s, CLS 0, and SEO, accessibility and best practices
  100 in all 12 runs.
- EN is under 200 ms, by less than the spread between runs.
- The medians are in `WEB-5-RESULT.md`, "Update: review fix R6"; the
  reviewer's sections are untouched.

**D6, pixel-identical: not met exactly.**
- Lossless CDP PNG captures of each tile on both builds, scrolled into
  view, compared pixel by pixel. At 1440 @1x, 4,315 and 3,483 of 72,150
  pixels differ (map, calendar), max channel delta 53 and 49. The shares
  are similar at 1440 @2x and at 390 @2x.
- A 4× zoom with a difference mask shows only the antialiased rims of the
  dots and cells differ. Fills, colours, positions and the box size
  (370 × 195.27 px at 1440) are identical, so there is no layout shift.
- The likely cause: Chrome rasterizes SVG-as-image differently from inline
  SVG when it scales the viewBox to a fractional box.
- I could not make an `<img>` match exactly. The owner judges the rims at
  QA; reverting `66606e38` restores the inline tiles.

**Commands and counts.**
- Site block on the final code: lint 0, typecheck 0, `bun run test`
  49 pass, 0 fail, build 0, `bun run size` 0 (178.6 KB).
- The scratch worktree is removed, and my servers on :3460, :3461 and
  :9555 are stopped. `git status` is clean.

### Handoff 5: developer (Opus), 2026-11-04, owner QA note 1 (R7)

**Commit:** `b2634f3d` WEB-5 review fix R7 (owner) (base `56f330d8`), pushed.

**What changed** (the RantAI site's rule, read in `rantai-page` and not
changed):
- **The 22 non-code monospace uses are sans:** the 21 `lk-mono` classes
  and the KPI tile's SVG `<text>`. They cover the eyebrows, labels, source
  pills, captions, the governance headers, cells and toggle label, the
  footer, the depth markers and gauge, the tile kinds, the chart labels and
  the scroll cue. The uppercase tracked labels keep their tracking, size
  and weight; in the screenshots they read as intentional.
- **The pipeline name `sales_nightly`** in the approval card stays
  monospace, through the base `code` rule and the system stack the RantAI
  site uses.
- **Governance amounts** use `tabular-nums`.
- **Geist Mono's woff2 and loader are removed.** Two font files are
  preloaded now, not three, and `.lk-mono` is gone.
- The tile SVGs have no text, so they were not regenerated;
  `tile-svg.test.ts` passes on the unchanged files.

**Verified.**
- Over CDP on `/` and `/id/`, the only element whose text is in a
  monospace family is that `<code>`, and the amounts' `font-variant-numeric`
  is `tabular-nums`.
- Screenshots of the sources, depths, governance and footer, EN and ID:
  `docs/plans/web-5-screens/r7-desk.jpg` (1440) and `r7-phone.jpg` (390).

**Commands and counts.**
- Site block: lint 0, typecheck 0, `bun run test` 49 pass, 0 fail,
  build 0, `bun run size` 0 (178.5 KB).
- No Lighthouse rerun, because no font file was added (one was removed).
- I served on :3460 and :9555 only, and both are stopped; :3400 was not
  touched. `git status` is clean.

### Handoff 6: developer (Opus), 2026-11-04, D14 storyboard (S1–S6)

**Commits** (`85c76d3c..c0e9dd95`, all pushed, in plan order):

| Commit | What |
| --- | --- |
| `388c83eb` | R1 follow-up: unique keys on the chat's switching `m` elements. React's dev build warned "two children with the same key" (found while building S1). |
| `e0b6c085` | S1 the dive |
| `e89f4c0f` | S2 the page-wide water |
| `a9bee695` | S3 the depth gauge |
| `f0b81418` | S4 the guide |
| `85187993` | S5 the ascent and the surfacing |
| `c0e9dd95` | S6 the measurement, in `WEB-5-RESULT.md` |

**S1, the dive** (evidence: `docs/plans/web-5-screens/d14-s1-dive-strip.jpg`).
- **Hero length.** The hero is 220 vh while the live lake runs
  (`DIVE.heroVh`). The DOM hero keeps 150 vh.
- **Camera path.** The camera eases to y 0.45 at 0.6 vh, then crosses
  slowly: 0.16 at 1.1 vh and −0.16 at 1.6 vh, about 0.6 units per viewport
  (it was about 2.6). It reaches −2.4 at 2.7 vh, in the sources (−5 m).
- **Underwater blend.** It follows the un-hopped path height, from 0.32 to
  2.36 vh, with at most 1.1 % per 1 % of a viewport. The `avoidSurface`
  band is halved, to 0.07.
- **The waterline.** A single water plane cannot show both sides, so the
  sky-to-ceiling flip is one frame. A waterline overlay driven by the
  engine (meniscus plus tint) rises up the screen, and the flip happens
  with 75 % of the screen already under it. A ripple spreads from the
  crossing.
- **The strip.** Eight frames lie between fully above (0.9 vh) and fully
  below (1.6 vh): 1.10, 1.20, 1.25, 1.30, 1.35, 1.40, 1.45 and 1.50.
- **The test-only override.** The frames were rendered with
  `NEXT_PUBLIC_WEB5_FORCE_WEBGL=1`, read only when NODE_ENV is
  "development" (`next dev` on :3461, a SwiftShader Chrome on :9556). The
  export drops it, and `scripts/check-bundle.mjs` fails if the flag's name
  is anywhere in `out/` ("not in the export" in every run since).

**S2, one water** (evidence: `d14-s2-edges.jpg`).
- **The layer.** `DepthBackdrop` is a fixed layer whose top and bottom
  colours follow the depth through CSS variables, written from a passive,
  rAF-throttled listener only when they change. Under reduced motion the
  colour steps per chapter.
- **The journey** is `JOURNEY` in `home-content.ts`, and the maths is in
  `lib/depth-journey.ts` (tested).
- **No-JS fallback.** The sections keep their own backgrounds only without
  JS (`.lk-fallback-bg`, dropped once `data-scene` is set).
- **Seams fixed:**
  - the assistant's old top fade is no-JS only;
  - its caustics fade in and out over the stage's first and last quarter;
  - the dashboards shore wash fades to transparent.
- **The evidence:** each section edge at mid-screen at 1440, under reduced
  motion and at 390, plus the live-lake seam from the gold slab into the
  assistant. There is no band at any edge.

**S3, the gauge** (evidence: `d14-s3-gauge-{desk,phone,small}.jpg`, 1440,
390 and 360, EN over ID, one frame per chapter).
- **Desktop:** a scale on the left edge, hidden inside the depths pin,
  where that section's own gauge with the slab labels stands.
- **Phone:** a hairline under the bar, with the reading at its right end.
  A reading in the bar's row would crowd 360 px.
- **Readings:**
  - hero 0, sources −5, Bronze / Silver / Gold −10 / −40 / −90 (as their
    labels), assistant −100;
  - dashboards −95 / −96, governance −45 / −46, open −12 to −14;
  - final 0;
  - EN and ID read the same.

**S4, the guide** (evidence: `d14-s4-guide.jpg`).
- The creature's mark, in a small disc, sits beside both assistant turns.
  There is no copy change.
- At 360×740 the pinned assistant stage already overflowed before S4: on
  `a9bee695` the heading was under the bar and the chat ran past the
  bottom. I fixed it with phone-only spacing. It now measures heading top
  / chat bottom 77 / 727 (EN) and 66 / 738 (ID); desktop is unchanged.

**S5, the ascent** (evidence: `d14-s5-ascent.jpg`,
`d14-s5-bubbles-detail.jpg`, and `t5-{reduced,nowebgl,nojs}.jpg`
re-taken).
- **Caustics** grow as the water gets shallower, and twelve CSS **bubbles**
  rise at the edges. They run only during the climb, are mounted after
  paint, and are hidden under reduced motion and without JS.
- **Reveals** already rise into place (their start offsets are downward),
  so they move with the ascent. Nothing was added.
- **The surfacing.** The final creature rises from 30 % lower as the
  section enters.
- **Fixed on the way:**
  - the stepped (reduced-motion) water showed the shallow depths colour at
    the assistant's start;
  - the final's grain drew an edge.
- **Fallbacks.** The three modes still read end to end.

**S6, the measurement** (`WEB-5-RESULT.md`, "Update: D14 S6").
- `bun run size`: 181.1 KB, with the scene 147.0 KB, deferred.
- Lighthouse, interleaved against R7 (`b2634f3d`), three rounds:

| Metric | EN R7 | EN D14 | ID R7 | ID D14 |
| --- | --- | --- | --- | --- |
| TBT median | 227 ms | 118.8 ms | 126 ms | 119.3 ms |

- LCP is 1.07 s, CLS 0, and SEO, accessibility and best practices 100 in
  all 12 runs.
- Every target is met. I read this as "no regression": the lower medians
  are within the VM's spread, and R7's EN median includes a 3,990 ms spike.

**Checks.**
- The site block on the final code (`85187993`'s code; S6 is docs only):
  lint 0, typecheck 0, `bun run test` 61 pass, 0 fail, build 0,
  `bun run size` 0 (181.1 KB).
- No new library and no per-scroll React state:
  - the backdrop, the gauge, the waterline and the final's rise write
    CSS variables or styles through refs;
  - the only setState calls are on mode or lit-depth changes, as before.

**Not verified, and limits:**
- **Not measured by Lighthouse:** the live-lake path, including the 220 vh
  hero and the waterline. This VM's Chrome has only SwiftShader, so it was
  checked visually with the dev-only override, and not for performance on
  a GPU.
- **Observed, not changed (outside D14):** at 360 px the English hero h1
  wraps to three lines ("Your data / has a / home now."). T7's check
  covered 1440 and 390.
- **Process:**
  - `next dev` writes `site/AGENTS.md`; I removed it each time, and it
    was never committed;
  - scratch worktrees (`a9bee695`, `b2634f3d`) are removed;
  - my servers on :3460, :3461, :9555 and :9556 are stopped, and :3400
    was not touched;
  - `git status` is clean.

### Handoff 7: developer (Opus), 2026-11-04, review 5 (R8–R10)

**Commits** (one per finding, in order, each cited at its fix site):

| Commit | Finding | What changed |
| --- | --- | --- |
| `fec50461` | R8 (BLOCKER) | The left-edge scale starts at 1360 px instead of `md`. The hairline is used below that. |
| `55396a17` | R9 (SHOULD-FIX) | `journeyFromBoxes` (pure, `lib/depth-journey.ts`) and `measureJourney` (`lib/measure-journey.ts`) are shared by the gauge and the water. The hero hold is the `holdSurfaceThroughHero` parameter. |
| `7edae5e8` | R10 (SHOULD-FIX) | `scene-mode.tsx` measures the first section on screen before and after writing `data-scene`, and `scrollBy`s the difference. |

**R8, measured** with `gaugeclear.ts` (CDP, built export on :3460). The gap
is from the marker's and reading's right edge to the first visible text in
each section:

| Width | Assistant | Dashboards | Governance | Open |
| --- | --- | --- | --- | --- |
| 768, 1024, 1280, 1359 | hairline, nothing at the left | hairline | hairline | hairline |
| 1360 | 37 px ("−100 m", the widest reading) | 45 px | 45 px | 45 px |
| 1440 | 77 px | 85 px | 85 px | 85 px |

- At 1320 the gap was 24 px only with a shorter reading, so 1360 is the
  first width that clears 24 px for every reading.
- Screenshots: `docs/plans/web-5-screens/r8-gauge-w1024.jpg` and
  `r8-gauge-w1280.jpg`, every chapter, EN over ID.
- The hairline readings match the storyboard in both languages:
  0, −5, −10 / −40 / −90, −100, −96, −45, −13, 0.

**R9, no change in behaviour.**
- Four new tests cover the pure part:
  - anchors from boxes, with depths and climb;
  - the hold adds exactly one anchor;
  - missing sections;
  - the one-pixel pin floor.
- `journeydump.ts` read these every 0.1 screen down the page, at 1280×800:
  - the gauge's `--lk-gauge-f`, reading and `data-in-depths`;
  - the water's four CSS variables.
- The output is byte-identical between `fec50461` and `55396a17`, with and
  without reduced motion (150 and 82 positions).
- Scene "none" only: this VM has no GPU. The hold in the live mode is
  covered by the unit test, *not verified* in a browser.

**R10, the probe** (`r10probe.ts`):
- Setup: `next dev` with the dev-only WebGL override, on SwiftShader
  Chrome, at 1280×800.
- `#governance` is held at y 64 until the mode leaves "pending".
- "full" then grows the hero from 1200 to 1760 px.

| Scroll anchoring | R10 | `scrollBy` | `#governance` top after | scrollY |
| --- | --- | --- | --- | --- |
| on | on | not called (difference 0) | 64 | 9308 → 9868 |
| off (`overflow-anchor: none` on html) | on | once, +560 | 64 | 9308 → 9868 |
| off | stubbed (control) | none | 624 | 9308 |

**Checks** on `7edae5e8`, the final code (this handoff is docs only),
run with `site-checks.sh` from a clean `.next`/`out`:
- `bun run lint` 0
- `bun run typecheck` 0
- `bun run test`: 65 pass, 0 fail
- build 0
- `bun run size` 0: 181.3 KB of 200 KB
- The dev flag's name is absent from `out/` (check-bundle).
- Lighthouse was not re-run, as asked.

**Not verified, and limits:**
- **R10 in other browsers:** not run in Safari or Firefox; this VM has only
  Chrome.
- **Observed, not changed (outside R10, for the reviewer to triage):** a
  deep link to `/#governance` lands about 1052 px short, in both the export
  and dev.
  - Sequence: the hash scroll is smooth (`scroll-behavior: smooth` on
    html), and while it runs, hydration grows `#dashboards` from 800 to
    1852 px (its rail).
  - The scroll ends at the old target, with `#governance`'s top at 1116
    instead of 64.
  - This is not the scene write. The R10 correction runs later and keeps
    whatever is on screen then.
- **Process:**
  - `next dev` wrote `site/AGENTS.md`; I removed it, and it was never
    committed;
  - my servers on :3460, :3461, :9555 and :9556 are stopped, and :3400
    was not touched;
  - `git status` is clean.

### Handoff 8: developer (Opus), 2026-11-04, review 6 (R11)

**Commit:** `a73ed156`, R11 (SHOULD-FIX), cited at the fix site in
`site/src/components/effects/scene-mode.tsx` (`landOnHash`).

**The choice: land on the hash after the settle.** I did not give the
dashboards their final height before hydration, because:
- that height is measured from the tiles and the fonts
  (`dashboards-scroller.tsx`), so a server-side copy would be a second
  formula to keep in step;
- without JavaScript it would add a screen of pinned scroll whose rail
  never slides;
- one landing after the settle also covers the live hero's later 70 vh.

**What it does:**
- It runs once per load, at the first mode other than "pending", after
  `document.fonts.ready` and one frame.
- It calls `scrollIntoView({ behavior: "instant" })` only if both hold:
  - `location.hash` names a `section`;
  - no wheel, touch, key or pointer input has come since hydration.
- The section's `scroll-mt-16` keeps it below the bar.

**Probe** (`r11probe.ts`, CDP, built export on :3460, 8 s after load,
section top in px):

| Viewport | `/#governance` | `/#open` | `/id/#open` |
| --- | --- | --- | --- |
| 1440×900 | 64 | 64 | 64 |
| 390×844 | 64 | 64 | 64 |
| 1440×900, `scrollIntoView` stubbed (control) | 1036 | 1036 | not run |
| 1440×900, live scene (`next dev`, dev-only override, SwiftShader, 25 s) | 64 | 64 | 64 |

- The control matches the reviewer's 1036.
- `/#` (an empty hash) scrolls nothing.

**The early scroller.**
- Setup: the CPU is throttled 8×, so that real input arrives while the mode
  is still "pending": a wheel of +300 at 1440, a touch drag at 390.
- In all six cases (three links, two viewports), `scrollIntoView` was
  never called, and the page stayed where the input took it.
- Without throttling, this VM settles before CDP can send input: the
  landing ran 1–3 frames into the smooth scroll.

**Checks** on `a73ed156`, from a clean `.next`/`out`:
- `bun run lint` 0
- `bun run typecheck` 0
- `bun run test`: 65 pass, 0 fail
- build 0
- `bun run size` 0: 181.5 KB of 200 KB
- Lighthouse not re-run, as asked.

**Not verified, and limits:**
- **Input before hydration is not seen,** since the listeners attach on
  hydration. Such a visitor can still be moved to the section once.
- **Browsers:** not run in Safari or Firefox.
- **Process:**
  - `next dev` wrote `site/AGENTS.md`; I removed it, and it was never
    committed;
  - my servers on :3460, :3461, :9555 and :9556 are stopped;
  - :3400, :3402, :3403 and :9557 were not touched;
  - `git status` is clean.

### Handoff 9: developer (Opus), 2026-11-07, review 8 (R12)

**Commit:** `3ff3eee5`, R12 (BLOCKER). It is cited at the fix sites:
- `hero-section.tsx` (`HeroCreature`)
- `lake-engine.ts` (`fitHeroToRoom`)
- `app/globals.css` (the `short` variant)

**The choice.** The copy is laid out below the creature, and the creature
scales into the room the copy leaves.
- **Layout.** Below `md` the hero stage is a column:
  - The creature moved out of the backdrop into its own slot
    (`data-hero-room`), which takes the free height above the copy.
  - The copy sits under it with a 16 px gap: the 12 px asked for, plus the
    bob's 0.6° tilt, which widens the drawing's box by about 1 px.
- **No change where it fits.** The gap above the creature (14 svh, at
  least 4.5 rem) and the creature shrink only when that height runs out.
  - The creature keeps its old place and size where there was room.
  - It stays under the copy's wash (`-z-[5]`).
  - From `md` up it is placed on the right as before.
- **The `short` variant.** It covers screens below `md` that are at most
  832 px tall: 360×800 and 375×812 are short, 390×844 is not.
  - The copy is set tighter: h1 3.25 rem, smaller gaps and lead, `pb-24`
    and a shorter scroll cue.
  - So the creature keeps a readable size, and both buttons stay on the
    first screen.
- **The 3D creature.** On a phone, if the 3D creature (waterline to the
  top of the dome) would end below the room, the camera is pulled back and
  the view shifted, so it fills the DOM creature's box. Otherwise the
  framing is unchanged.

**Measured** (`r12probe.ts`, CDP, built export, scene "none"). The
creature's box includes the drawing's bob, sampled over one 4 s cycle.

| Screen | EN gap | ID gap | Buttons end (both) |
| --- | --- | --- | --- |
| 360×640 | 14 px | 15 px | 544 of 640 |
| 360×740 | 84 px | 62 px | 644 of 740 |
| 375×667 | 14 px | 15 px | 571 of 667 |
| 390×844 | 45 px | 17 px | 716 of 844 |
| 412×915 | 96 px | 68 px | 787 of 915 |

Extra screens, all on the first screen and clear by at least 14 px:

| Screen | EN gap | ID gap |
| --- | --- | --- |
| 320×568 | 15 px | 15 px |
| 360×800 | 135 px | 113 px |
| 375×812 | 139 px | 117 px |
| 393×852 | 51 px | 23 px |
| 414×736 | 55 px | 55 px |

On the build before the fix (`63f09a87`), 360×800, 360×900 and 375×812
also overlapped in EN, and the overlap was not limited to the listed sizes.

**Nothing changes at 390×844 and up.** `herodiff.ts` compares lossless
PNGs with animations off, against `63f09a87`'s build. It finds 0
differing pixels at:
- 390×844, 412×915, 430×932, 768×1024, 1024×768 and 1440×900, in EN and ID;
- 390×844 and 1440×900 with reduced motion.

The first try differed by about 700 anti-aliased edge pixels at 390×844.
The creature's box is now positioned exactly as before (`left-1/2`,
`-translate-x-1/2`), which brought that to 0.

**The live 3D hero** (`next dev`, dev-only override, SwiftShader Chrome):
- Checked by screenshot at 360×740 (EN, ID), 375×667 (EN, ID), 360×640
  and 320×568.
- At 320×568 the new framing takes over, and the dome fits the DOM
  creature's box.
- The DOM boxes measure the same as in the export.
- The 3D dome's edge itself is checked by eye, *not measured* in pixels.

**Evidence** in `docs/plans/web-5-screens/`:
- `r12-hero-short-dom.jpg`: 4 before, then 4 after, at 360×740 and
  375×667, EN and ID;
- `r12-hero-short-live.jpg`: the live hero.

**Checks** on `3ff3eee5`, from a clean `.next`/`out`:
- `bun run lint` 0
- `bun run typecheck` 0
- `bun run test`: 65 pass, 0 fail
- build 0
- `bun run size` 0: 181.5 KB of 200 KB
- Lighthouse not run, as asked.

**Not verified, and limits:**
- **Not tried on real phones,** nor in Safari or Firefox. The probe uses
  Chrome mobile emulation.
- **Small creature on the smallest screens.** On 320×568 the DOM creature
  is 72 px, because the room left above a two-line headline is small.
- **Process:**
  - The worktree was recreated by the planner, so I ran
    `bun install --frozen-lockfile` with bun 1.3.12 in `site/`; the
    lockfile is unchanged.
  - `next dev` wrote `site/AGENTS.md`; I removed it, and it was never
    committed.
  - My servers on :3460, :3461, :3462, :9555 and :9556 are stopped.
  - :3400 and :9557 were not touched.
  - `git status` is clean.

## 8. Review (planner appends)

### Review 1: planner, 2026-11-04, at `e78fa90b`

**Re-run by the reviewer** (rule 7; site from a clean `git archive` of `e78fa90b` into a scratch
folder, fresh `bun install --frozen-lockfile`):

- site: lint exit 0; typecheck exit 0; `bun run test` 45 pass, 0 fail; build exit 0;
  `bun run size` exit 1 (209.0 KB on both pages; noModule 39.6 KB listed; scene 146.8 KB in one
  deferred chunk); a build without `NEXT_PUBLIC_SITE_URL` stops with the `SiteUrlError` message;
  `out/index.html` is `lang="en"`, `out/id/index.html` is `lang="id"`, no `out/en/`.
- `python3 ops/release/test_release_workflow.py`: 118 tests OK.
- `scripts/ci/tests/test_detect_change_scope.sh`: 40 passed; `test_required_gate.sh`: 62 passed.
- Every claim in `claims.ts` checked against `BACKLOG.md`: each cited row has the status it says
  (Released or In Acceptance; `BI-16` Building, cited only for "your own map boundaries"). None of
  the "do not add" items appear. `README.md#quickstart` and the user guide path exist.
- `src/proxy.ts` has no session redirect, so `/landing` stays a server-side 404 after it leaves
  `PUBLIC_PREFIXES`, and release.yml's install proof still holds.

**Findings:**

- **R1 BLOCKER: the initial JS is 209.0 KB against 200 KB, so the CI site job is red** and the PR
  would add a failing check (AGENTS.md step 6). A cut without a design change exists that the
  handoff does not list: motion's async `LazyMotion`,
  `features={() => import("./dom-animation").then((r) => r.default)}`, which moves the
  `domAnimation` feature bundle out of the first load. (In the build, chunk `3_fhy…`, 30.9 KB,
  holds motion's animation code.) Conditions:
  - the hero `h1` and the CTAs must not wait on the features chunk (LCP);
  - if the features chunk never loads, revealed content must still become visible. `Reveal`
    starts at `opacity: 0` once `js` is set, so a failed chunk must not leave the page blank:
    a CSS fallback, or the `js` class added only once the features have loaded;
  - the T5 screenshots (no-JS, no-WebGL, reduced motion) are re-taken and unchanged.

  If the page is still over 200 KB after that, the RESULT file records the new number and the
  owner decides. The budget is not changed in the PR.
- **R2 SHOULD-FIX: label in name (WCAG 2.5.3) on the EN/ID links.** The fix is known and small;
  apply it rather than record it. The accessible name must contain the visible "EN"/"ID".
- **R3 SHOULD-FIX: `codeRepository` is not a `SoftwareApplication` property** in schema.org (it
  belongs to `SoftwareSourceCode`), so validators warn. Either drop it or add a
  `SoftwareSourceCode` node with `codeRepository` and `targetProduct` pointing at the
  application's `@id`. Update `structured-data.test.ts`.
- **R4 SHOULD-FIX: at 360×740 the sources headline is clipped under the top bar.** 360 px is the
  most common Android width, so this is in the audience, not an edge case. Fix it with spacing
  only (D6), and add a CDP screenshot at 360×740, EN and ID.

**Confirmed (no change):**
- the routing shape (`app/(en)` plus `app/[locale]` with only `id`);
- plain-href language links and no next-intl navigation or client provider;
- OG cards as build-time route handlers;
- Query Studio as the last sentence of the assistant lead;
- 19 source pills;
- the dependencies beyond D1, which the moved code needs;
- the manifest name.

**TBT:** not a finding yet. The VM's load (11–14) makes single runs swing 206–1642 ms. The
reviewer's Lighthouse run happens on the commit that closes R1, which also shrinks what is
hydrated, and is recorded under "Reviewer's run" in the RESULT file.

### Review 2: planner, 2026-11-04, at `61b5aa26`

**Re-run by the reviewer:**
- **Site,** from a clean `git archive` with a fresh install: lint 0; typecheck 0; 46 pass,
  0 fail; build 0; `bun run size` 0, at 181.2 KB on both pages.
- A build with no URL set stops with `SiteUrlError`. The pages are `lang="en"` and `lang="id"`.
- **Lighthouse,** three runs per locale: recorded in `WEB-5-RESULT.md` under "Reviewer's run 1".

**R1–R4: closed.**
- **R1:** 181.2 KB.
  - The failed-chunk and reduced-motion conditions are met as the commit describes.
  - No reveal sits in the hero, so the switch to `motion-ready` after load cannot flash the
    first screen.
- **R2:** label in name passes in all six runs.
- **R3:** the `SoftwareSourceCode` node, with its test.
- **R4:** the 360×740 screenshot, EN and ID, shows the headline clear of the bar.

**Decision (planner): `framer-motion` 14.9.1 replaces `motion` ^14.9.0 in D1. Accepted.**
- It is the React package that `motion/react` re-exports, at the same version, from the same
  maintainers, so there is no new vendor and no API change.
- The measured reason: the `motion/react` wrapper keeps the full `motion` component.
- D1 is read with this substitution, and the exact pin stays.

**New finding:**
- **R5 SHOULD-FIX (before owner QA): the section files are client components, against D5.**
  - D5: "Sections that only render text are server components. Motion and scroll pieces are
    client leaves." Seven of the eight `components/home/*-section.tsx` files (all but `open-section`) start
    with `"use client"`.
  - The reviewer's runs put the EN TBT median at 225 ms against 200 ms. The longest task is
    react-dom's hydration in every run (234 ms in EN 3).
  - Ask:
    - move `"use client"` down to the leaves that need it (`Reveal`, the scroll-driven pieces,
      the tilt cards, the role toggle, the chat illustration), so headings, lead text, lists and
      captions render on the server and are not hydrated as client components;
    - no visual change (D6);
    - re-take the T5 screenshot set;
    - run Lighthouse three times per locale and add the medians to the RESULT file as a dated
      update.
  - If the EN median is still over 200 ms after the split, write the number and the remaining
    long tasks. The owner then decides; the target is not lowered.

### Review 3: planner, 2026-11-04, at `382c190d`

**Re-run by the reviewer:**
- Site, from a clean `git archive` with a fresh install: lint 0, typecheck 0, 46 pass, build 0.
- `bun run size` 0, at 178.6 KB.
- Lighthouse, interleaved against `61b5aa26`: recorded in RESULT under "Reviewer's run 2".
  - EN TBT 213 ms after R5 against 215 ms before.
  - ID TBT 208 ms after against 209 ms before.

**R5: closed.**
- The sections are server components with client leaves, as D5 asks.
- TBT on equal terms is unchanged. The developer's 284 ms was the VM's load.
- The dashboards-rail regression from R1 was found and fixed in R5. The fix keeps refs off
  animated elements.

**New finding:**
- **R6 SHOULD-FIX: render the two static dashboard tiles as images,** per the developer's own
  suggestion in Handoff 3.
  - The dot map and the calendar heatmap are about 420 of the page's 1,088 elements.
  - Each becomes one `<img>` of an SVG file in `public/`, with the tile's `aria` text as `alt`.
    The look is pixel-identical, and D6 applies.
  - Then measure interleaved against `382c190d`, as in "Reviewer's run 2": two `npx serve`
    ports, three rounds of EN and ID.
  - Add the medians to RESULT as a dated update.
  - If EN is still over 200 ms, write the number. The owner decides; the target is not
    lowered.

### Review 4: planner, 2026-11-04, at `910a4be3`

**Re-run by the reviewer** (clean `git archive`, fresh install):
- lint 0, typecheck 0;
- 49 pass, 0 fail;
- build 0;
- `bun run size` 0, at 178.6 KB;
- `out/tiles/*.svg` are served;
- `lang="en"` and `lang="id"` are set.

**R6: closed.**
- The developer measured, interleaved, EN TBT at 197.8 ms and ID at 140.1 ms. The EN margin is
  smaller than the run-to-run spread; RESULT says so.
- **Decision (planner): the antialiasing differences are accepted under D6.** They sit only on
  the rims of the dots and cells. Fills, colours, positions and the tile box are identical, so
  nothing moves and no one can see the change at normal size.
- The owner can still ask for a revert at QA (`66606e38`).

**No open finding.** Next steps: owner QA on the R6 build (served on :3400), then CI green, then
the PR marked ready.

### Owner QA note 1, 2026-11-04

> "sama kayak rantai landing page, aku gasuka font monospace dipake kalo bukan untuk terminal"

This changes D6 for one thing: type.
- **The rule:** monospace is used only for code or terminal text, as on the RantAI site, where
  `globals.css` sets `--font-mono: var(--font-sans)` and a real monospace stack appears only in
  `.code-frame pre code`.
- **R7 (owner):** every `lk-mono` use that is not code becomes the sans face. That covers the
  eyebrows, labels, source pills, captions, table headers and cells, the footer, the depth
  markers and the tile kinds.
  - The one code identifier, the pipeline name in the approval card, stays monospace.
  - Numbers in the governance table use `tabular-nums` in sans.
  - If Geist Mono is then only used for that one `<code>`, drop the font file and use a system
    monospace stack, as the RantAI site does.

### Review 5: planner, 2026-11-04, at `1f365b3a` (D14, S1–S6)

**What the reviewer re-ran** (clean `git archive`, fresh install):
- lint 0, typecheck 0;
- tests: 61 pass, 0 fail;
- the build exits 0, and a build without `NEXT_PUBLIC_SITE_URL` fails, as it should;
- `bun run size` exits 0: 181.1 KB initial JS for EN and for ID. The scene chunk is 147.0 KB and deferred. The test-only override is not in the export.

**Read against §9.**
- **The keys match the storyboard.** `JOURNEY` (`home-content.ts`) reads 0 → −5 → −10 / −40 / −90 → −100 → −50 → −15 → −3 → 0.
- **No per-scroll React state.** The water, the gauge and the surfacing write CSS variables or a transform through refs, from passive listeners throttled to one per animation frame.
- **The override cannot ship.** It is behind `NODE_ENV === "development"`, and `check-bundle.mjs` guards it.
- **The changed tests still check the original intent.** The one that said "crosses in the first viewport" was rewritten for the slow dive, and its comment in the code says why. The `avoidSurface` minimum went from 0.13 to 0.065, and it still clears the 0.05 near plane.
- **Evidence:**
  - the dive strip has eight frames between above and below;
  - the section edges show no colour band;
  - the gauge was shot at 1440, 390 and 360, in EN and ID;
  - the guide fits a 360×740 screen;
  - the ascent set and the three fallback sets were re-taken.

**Probes the reviewer ran.** The build was served on :3402, in headless Chrome with no GPU, so the page ran in the `none` scene mode. The probe script is in the planner's scratchpad and is not committed.

**R8: BLOCKER, the desktop gauge covers text from 768 to about 1280 px.**
- **What happens:** the scale is `fixed left-8 w-28` from the `md` breakpoint up. Its marker and reading span x 35–92 px.
- **What was measured**, against the left edge of the first visible text:

  | Width | Dashboards | Governance | Open |
  | --- | --- | --- | --- |
  | 1024 | 22 px | 23 px | 23 px |
  | 800 | 22 px | 23 px | 24 px |
  | 1280 | 94 px | 95 px | 96 px |

  - At 1024 and 800, the text starts at x 22–24, so the reading lies on top of it.
  - At 1280 the gap is 2 px.
  - At 1440, the text starts at x 173 or further right.
- **Why it matters:** these are tablet and small-laptop widths, and S3's screenshots covered only 1440, 390 and 360.
- **Fix:** keep the left scale only where it has room, and use the phone hairline below that width. The cut-off is the developer's choice, measured so that the reading clears the container's text by at least 24 px in every section.
- **Acceptance:**
  - the same check at 768, 1024, 1280 and 1440 reports no overlap;
  - screenshots at 1024 and 1280.

**R9: SHOULD-FIX, the journey is measured twice (rule 4).**
- **What happens:** `depth-backdrop.tsx` and `depth-gauge.tsx` each build the chapter anchors from the page. Each has its own copy of the Gold pin-end key and the surface key.
- **The copies already differ.** Only the gauge holds 0 m through the hero's pin.
- **Why it matters:** a later change to one copy will make the water and the gauge disagree about the depth.
- **Fix:** one function measures the page and returns `{ anchors, depths, climb }`, and both components use it. Any difference in what the two components need, such as the hero hold, becomes a parameter, with a comment saying why.
- **Acceptance:**
  - there is one measuring function;
  - the existing tests pass;
  - a test covers the pure part, which turns section boxes into anchors.

**R10: SHOULD-FIX, the hero grows by 70 vh after the page loads.**
- **What happens:** `html[data-scene="full"] #hero { height: 220vh }` applies only once the mode settles, which is after the first paint. A visitor who reloads mid-page or follows a deep link is already scrolled when it applies.
- **What Chrome does:** scroll anchoring keeps the view in place. The probe set `data-scene="full"` with `#governance` at the top of the screen. Its top stayed at 0, and `scrollY` grew by 630 px, which is the change in the hero's height.
- **What is not verified:** Safari and other engines. A browser that does no scroll anchoring would move the page by 70 vh under the visitor.
- **Fix:** where `data-scene` is written, take the top of a section on screen before the write and again after it. If it moved, `scrollBy` the difference. In Chrome the difference is already 0, so this is a no-op there.
- **Acceptance:**
  - the same probe with scroll anchoring off (`overflow-anchor: none` on `html`) shows the section's top unchanged.

**Notes. These are not findings.**
- **The hero headline at 360 px.** The developer saw that the English headline wraps to three lines at 360 px. This is outside D14, and it goes to the owner's QA.
- **Lighthouse:**
  - The developer's interleaved rounds against R7 are in RESULT: EN TBT 118.8 ms and ID 119.3 ms, LCP 1.07 s, CLS 0, and 100 in every category.
  - The reviewer will re-run them on the final commit after R8–R10. A re-run now would be replaced anyway.
  - As the handoff says, these runs never load the live 3D scene, because this VM has no GPU.

**Open: R8 (BLOCKER), R9 and R10 (SHOULD-FIX).**

### Review 6: planner, 2026-11-04, at `59055f3e` (Review 5 fixes)

**What the reviewer re-ran** (clean `git archive`, fresh install):
- lint 0, typecheck 0;
- tests: 65 pass, 0 fail;
- the build exits 0, and a build without the site URL fails, as it should;
- `bun run size` exits 0: initial JS is 181.3 KB for EN and for ID, and the override is not in the export.

**R8: closed.**
- The overlap probe found no overlap at 768, 1024, 1280, 1360 or 1440.
- Below 1360 the scale is hidden and the hairline shows. In `r8-gauge-w1024.jpg` the hairline sits under the bar, clear of the text.

**R9: closed.**
- One `measureJourney`, built on the pure `journeyFromBoxes`, which has tests.
- The hero hold is a parameter, with a comment saying why it is there.

**R10: closed.**
- The view is measured before and after `data-scene` is written and kept in place.
- The developer's control run, with scroll anchoring off and the fix disabled, shows the 560 px move that the fix prevents.
- Safari and Firefox are not verified; this VM has only Chrome.

**R11: SHOULD-FIX, a deep link lands short of its section. It predates D14.**
- **What happens:** on the reviewer's run, at 1440×900, 8 s after load:
  - `/#governance` leaves `#governance` at y 1036, not at the top;
  - `/#open` leaves `#open` at y 1036.
- **Same on R7.** The R7 build (`6b1566c9`) gives the same numbers, so this is not a D14 regression.
- **Cause** (developer, Handoff 7): the browser scrolls to the hash smoothly, and meanwhile hydration grows `#dashboards` from 800 to 1852 px.
- **Why it matters:** in-page navigation from the top bar runs after hydration and is not affected. A shared or README link to a section is affected, and it lands on the wrong screen.
- **Fix:**
  - once, after the page has settled, if `location.hash` names a section and the visitor has not scrolled since load, scroll it into place with `behavior: "instant"`;
  - or give the dashboards scroller its final height before hydration.

  The developer picks one and says why. No new library.
- **Acceptance:**
  - `/#governance`, `/#open` and `/id/#open` put their section's top within 80 px of the top of the screen at 1440×900 and 390×844;
  - a visitor who scrolls before the page settles is not moved.

**Open: R11 (SHOULD-FIX).** After that: Lighthouse interleaved against R7 on the final commit, owner QA, CI, then the PR marked ready.

### Review 7: planner, 2026-11-04, at `1e5b0bf1` (R11)

**What the reviewer re-ran** (clean `git archive`, fresh install):
- lint 0, typecheck 0;
- tests: 65 pass, 0 fail;
- the build exits 0;
- `bun run size` exits 0: initial JS is 181.5 KB for EN and for ID, and the override is not in the export.

**R11: closed.**
- **Deep links land on their section.** On the reviewer's run, 8 s after load, each section's top sits at y 64, which is the top bar's height:
  - at 1440×900: `/#governance`, `/#open` and `/id/#open`;
  - at 390×844: `/#governance` and `/id/#open`.
- **How:** `landOnHash` runs once per load, only for a `section`, and only when there has been no wheel, touch, key or pointer input since hydration.
- **Not verified:**
  - Input before hydration is not seen, as Handoff 8 says. A visitor who scrolls that early can be moved to the section once. This is accepted, because it happens only on a deep link and lands where that link points.
  - Safari and Firefox were not tested.

**Lighthouse** ("Reviewer's run 3" in `WEB-5-RESULT.md`, interleaved against R7):
- TBT medians: EN 163.6 ms, ID 136.6 ms;
- LCP about 1.07 s, CLS 0;
- accessibility, SEO and best practices 100 in all twelve runs;
- every spec target is met. The difference from R7 is within the run-to-run spread.

**No open finding.** Next: the owner's QA on this build (served on :3400), then CI green, Dependency review included, then the PR marked ready.

### Review 8: planner, 2026-11-07, at `390a482f` (owner's question: is it safe on phones?)

**What the reviewer ran:**
- **Screenshots** of fifteen points down the page, in EN and ID, at 390×844 and 360×740. These are the reviewer's own, not committed.
- **A horizontal-overflow check** at every half screen of scroll, at 360×740, 375×667, 390×844 and 412×915, in EN and ID.
- **Mode:** the DOM hero (`scene: none`), which is what a phone without fast WebGL gets.

**What holds:**
- No page scrolls sideways at any of the four sizes, in either language.
- Every section reads at 360 and at 390.
- The assistant stage fits.
- The gauge hairline and its reading sit under the bar.

**R12: BLOCKER, on short phone screens the hero's creature sits under the eyebrow and the headline.**

| Screen | Language | Creature's bottom edge | Eyebrow's top edge | Headline's top edge |
| --- | --- | --- | --- | --- |
| 360×740 | EN | y 267 | y 139 | y 175 |
| 360×740 | ID | y 267 | y 212 | y 248 |
| 375×667 | EN | y 264 | y 93 | y 129 |
| 375×667 | ID | y 264 | y 139 | y 175 |
| 390×844 | EN | y 295 | y 344 | — |
| 412×915 | EN | y 315 | y 415 | — |

- On the short screens the text overlaps the creature by 55–171 px. It is clear at 390×844 and at 412×915.
- **Cause:** the hero copy is centred in the 100 svh stage, while the creature has a fixed place and size. On a short screen, the centred copy rises into it.
- **Why it matters:** the first screen a phone visitor sees is the hero. 360×740 is a common Android size, and 375×667 is the iPhone SE.
- **Fix:** on short screens the creature and the copy share the stage without overlapping. For example, the creature scales with the stage's height, or the copy is laid out below the creature instead of centred. The headline may still wrap. No new library, and no change at 390×844 and up.
- **Acceptance:**
  - at 360×640, 360×740, 375×667, 390×844 and 412×915, in EN and ID, the creature's box ends at least 12 px above the eyebrow;
  - both buttons stay on the first screen;
  - screenshots at 360×740 and 375×667, in EN and ID;
  - the live 3D hero is checked with the dev-only override at 360×740.

**Not measured on a phone:** the live 3D lake. This VM has no GPU. A phone whose WebGL2 passes `hasFastWebGL` runs the scene. That scene chunk (147 KB gzip) loads only after the page is usable, and no Lighthouse number here includes it.

**Open: R12 (BLOCKER).**

### Review 9: planner, 2026-11-07, at `89e049e2` (R12)

**What the reviewer re-ran** (clean `git archive`, fresh install):
- lint 0, typecheck 0;
- tests: 65 pass, 0 fail;
- the build exits 0;
- `bun run size` exits 0: initial JS is 181.5 KB, and the override is not in the export.

**R12: closed.**
- **The reviewer's probe** (built export, DOM hero, the creature's box `[data-hero-creature]` against the eyebrow), EN / ID:

  | Screen | Gap, EN / ID | Buttons end |
  | --- | --- | --- |
  | 360×640 | 16 / 16 px | y 544 of 640 |
  | 360×740 | 85 / 63 px | y 644 of 740 |
  | 375×667 | 16 / 16 px | y 571 of 667 |
  | 390×844 | 46 / 19 px | y 716 of 844 |
  | 412×915 | 97 / 70 px | y 787 of 915 |

- **No horizontal overflow** at any of these sizes.
- **Before and after** (`r12-hero-short-dom.jpg`): the creature sits above the copy on the short screens.
- **The live 3D framing.** It changes only where the 3D creature would reach the copy. It was checked by eye by the developer under SwiftShader, not by the reviewer. A phone with a GPU is still not verified.
- **Lighthouse:** "Reviewer's run 4" in `WEB-5-RESULT.md`. TBT medians are EN 105.2 ms and ID 155.5 ms, LCP is about 1.07 s and CLS 0. Every target is met.

**No open finding.** Next: the owner's QA on this build (:3400), then CI on `89e049e2` or later, then the PR marked ready.

## 9. Addendum: the story, top to bottom (owner, 2026-11-04)

The owner's QA asked:

> "ini landing page udah ada storynya kah? kek user pas buka dari atas sampe bawah tu kek ada
> ceritanya, jadi ga yang asal muncul muncul ajah"

On the hero-to-dive transition, the owner said it is "too sudden", not laggy. The owner also
said: "Dikerjain aja semuanya di PR ini." This addendum widens D6 for this PR. The page may now
change how it moves between sections. The sections, copy structure, palette and claims stay.

### Where the page is today

- The headlines already tell a dive and a return: the surface, "Everything flows in", three
  depths at −10 / −40 / −90 m, "Ask, don't dig", "Answers come back up", "Guarded by default",
  "Yours to keep" and "Come on in. The data's fine."
- The picture tells it only from the hero to the depths (the 3D `SceneZone`).
- From the assistant on, each section has its own flat background and a generic fade-up reveal.
  Nothing on screen says where the visitor is in the dive, so the second half reads as cards
  that "just appear".
- The dive itself is short. In `scroll-timeline.ts` the camera goes from y 1.15 to 0.05 over
  0.42 vh, then to −2.4 by 1.0 vh. It crosses the surface in a few dozen pixels of scroll, with
  a 0.28-unit hop (`avoidSurface`), and the sky-to-underwater colour swap happens in the same
  stretch. That is the "mendadak" moment.

### D14. One journey: down to the deepest point, then back up to the surface

**Storyboard.** Depth is the thread. The ascent mirrors the descent.

| # | Section | Depth shown | What the visitor sees change |
| --- | --- | --- | --- |
| 1 | Hero | 0 m, surface | Night lake, the creature, the moon (as today). |
| 1→2 | The dive | 0 → −5 m | **New, slower dive.** The camera lingers at the waterline in a half-above, half-below view before going under. The colour, fog and light change over a longer stretch, and the headline fades as the surface closes overhead. |
| 2 | Sources | −5 m | Pills fall in like droplets (as today). |
| 3 | Depths | −10 / −40 / −90 m | Bronze, Silver and Gold slabs (as today). The gauge now matches the slab labels. |
| 4 | Assistant | −100 m, the deepest point | The darkest water. The creature is the guide: it appears beside the chat as the assistant's face. "Ask, don't dig": you don't have to dive yourself. |
| 5 | Dashboards | −100 → −50 m, ascending | "Answers come back up." The water lightens with scroll and a few bubbles rise. The tile rail slides as today. |
| 6 | Governance | −50 → −15 m | Lighter water; the masked table as today. |
| 7 | Open source | −15 → −3 m | Near the surface: moonlight caustics return. |
| 8 | Final | 0 m, surfacing | The creature rises through the surface into the night sky (the existing final scene). The gauge reads the surface again. |

**Mechanics:**

- **One background for the whole page, driven by scroll.** Sections 4–8 lose their own opaque
  backgrounds and sit on one fixed layer. That layer's colour follows the depth: the hero's
  night, then the abyss at the assistant, then lighter water on the way up, then the final's
  night sky. The section edges no longer show as bands.
  - The layer is driven by a motion value or a CSS variable written through a ref. There is no
    React state per scroll event (D5).
  - The depths section (`SceneZone`) keeps the 3D canvas. The fixed layer starts where the
    canvas fades and matches its last colour, so there is no seam.
- **A depth gauge, visible the whole way down and up.** On desktop it is a thin vertical scale
  on the left edge with the current depth in metres and a marker for each chapter. On phones it
  is a small reading in the top bar's row, or a hairline at the top. The developer picks the
  one that does not crowd 360 px and shows both in the evidence.
  - It is `aria-hidden`. The chapter names it shows are already the section headings.
  - The depth values are data, in `home-content.ts`. Labels such as "Surface" / "Permukaan"
    are messages in both languages.
- **The dive (1→2).**
  - Give it more scroll: the hero section grows, with a number chosen by the developer and
    shown in the evidence.
  - Add a key at the waterline so the camera slows there (PCHIP keeps it monotone, with no
    bob). Spread the `underwater` / fog / colour change over that longer span instead of a short
    smoothstep.
  - Optional, if cheap: a ripple ring at the crossing.
  - Keep `avoidSurface`, but the hop must not be visible as a jump. If it still is, the
    half-above view should hide it.
- **The creature as guide (4, 8).**
  - In the assistant section, the chat's assistant side gets the creature's mark (the existing
    `MascotSvg`, small), so the guide is the same character the visitor met at the surface.
  - In the final, the existing rising creature completes the return.
- **The ascent (5–7).**
  - A few bubbles rise at the edges, in a CSS animation or a tiny canvas. They are deferred like
    the language offer and stop under reduced motion.
  - Caustic light grows stronger as the depth gets shallower.
  - The section reveals keep their content but move in the direction of travel: on the way up,
    content rises in with the bubbles. They are not new elements.
- **Fallbacks keep the story.**
  - Without WebGL, the fixed layer and the gauge still run, so the journey reads without the
    3D scene.
  - Under reduced motion, the gauge and the colours change per section with no animation, and
    there are no bubbles.
  - Without JS, the page reads top to bottom as today, with section colours, and no gauge is
    needed.

**Limits:**
- Every spec target still applies: initial JS ≤ 200 KB, TBT ≤ 200 ms, LCP, CLS, SEO and
  accessibility.
- The story adds no library. Anything not needed for the first screen is deferred.
- No new section and no new claim (D7).
- The copy may change only for gauge labels and, if needed, one line of the assistant's lead
  linking it to "the deepest point". That line is in both languages.

### Tasks S1–S6 (after R7, one commit each, in order)

1. **S1 The dive.** A longer hero, a waterline key, and the colour, fog and light spread over
   it.
   - *Accept:* a CDP frame strip of the crossing. Because the VM has no GPU, run with a
     test-only override of `hasFastWebGL`: a local patch or an env flag read only in
     `next dev`, never in the exported build.
   - There are at least five frames between "above" and "below", and no single scroll step
     changes the sky to underwater.
2. **S2 The page-wide depth background** for sections 4–8, with the seam at the end of
   `SceneZone`.
   - *Accept:* full-page CDP screenshots, 1440 and 390, show no colour band at any section edge.
3. **S3 The depth gauge.**
   - *Accept:* screenshots at 1440, 390 and 360, EN and ID, at each chapter. The reading matches
     the storyboard.
4. **S4 The guide:** the creature's mark in the assistant chat. Copy line only if needed.
5. **S5 The ascent:** bubbles, caustics and the reveal direction for 5–7, and the final
   surfacing tie-in.
   - *Accept:* the no-WebGL, reduced-motion and no-JS screenshot sets still read end to end.
6. **S6 Measure.**
   - Run `bun run size`.
   - Run Lighthouse interleaved against the R7 commit, three rounds, EN and ID.
   - Record the results in RESULT as a dated update.
   - A miss is written with its number, not lowered.

The owner reviews the result in QA, which can send any of S1–S5 back.
