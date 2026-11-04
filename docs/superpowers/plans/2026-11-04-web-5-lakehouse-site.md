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
    client leaves." All nine `components/home/*-section.tsx` files except `open-section` start
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
