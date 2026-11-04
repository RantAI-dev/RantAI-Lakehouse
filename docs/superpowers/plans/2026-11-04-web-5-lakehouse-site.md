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

## 8. Review (planner appends)
