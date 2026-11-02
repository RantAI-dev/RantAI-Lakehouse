# WEB-2: the landing page is not served by an installed Lakehouse

Backlog `WEB-2` (Task, Business, P2). Base: `main` at `9c066426`. Planner: Claude Opus.
Developer: another agent (`AGENTS.md`).

## 0. What is wrong, with evidence

The product owner decided on 2026-11-02 that the landing page (`WEB-1`) is a separate product
page and must not be available on an installed Lakehouse
(`docs/core/features/landing-page.md:164-167`, "QA-1 decisions"). Today every install serves
it:

- **The route is part of the console app.** It lives in `src/app/landing/page.tsx`, which renders
  `src/features/landing/landing-page.tsx`. Nothing gates it: there is no setting, no environment
  variable and no build flag.
- **It is public by design.** `src/features/auth/auth-provider.tsx:29` puts `/landing` in
  `PUBLIC_PREFIXES`, so a signed-out visitor sees it, and `app-frame.tsx:34` renders it without
  the console chrome. `src/proxy.ts` has no `/landing` branch; it only sets the CSP.
- **Every install builds it in.** The console image is built from the repository root with
  `Dockerfile.frontend`, which copies everything (`COPY . .`, `:27`) and runs `bun run build`
  (`:45`). The same Dockerfile is used:
  - by the release (`.github/workflows/release.yml:273-282`, dry run, and `:869-878`, real
    release);
  - by the staging deploy (`deploy-staging.yml:207`);
  - by compose's `console` service (`docker-compose.yml:781-798`).

  `.dockerignore` decides what that build sees, and it does not exclude the landing page
  (`:41-52`).
- **The build pulls a font from Google for it.** `src/features/landing/landing-fonts.ts:9` uses
  `next/font/google` (`Instrument_Serif`), which `next build` fetches at build time. This is the
  landing page's own font. Excluding the page also drops that build-time fetch from the console
  image build. Whether the console's root layout fetches its own fonts the same way is not
  checked here.
- **The release's install proof checks only `/`.** `release.yml:1414-1420` curls
  `http://127.0.0.1:3000/` on the installed bundle, and nothing proves `/landing` is absent.

## 1. Decisions

- **D1. The landing page is left out of the console image, by `.dockerignore`.** Two lines,
  `src/app/landing` and `src/features/landing`, in the block "The parts of the repository this
  image does not serve" (`:41`), with the reason (`WEB-2`, owner 2026-11-02). Every image built
  from the root then has no `/landing` route: the release, staging and compose. A visitor to
  `/landing` on an install gets the console's page-not-found. The path is still under a public
  prefix, so it shows without the chrome and without a sign-in redirect, as `WEB-1-AC16`
  already describes for `/landing/anything`.
  - *Why this, not an env var:* a runtime switch (`notFound()` unless `LANDING_PAGE=true`)
    still ships the page and its bundles in every image, and one wrong variable on a customer
    host turns it on. That is not "must not be available".
  - *Why not a build arg:* a build arg is the same switch moved to build time. Every build site
    (three of them) must pass it correctly, and the default must be off.
  - *Why not delete it from the repository:* it has no other home yet (D5), and deleting it now
    loses the page while the owner decides.

  `.dockerignore` already owns "what the console image is made of", so it is the place that
  needs no new mechanism. *(planner default)*
- **D2. The source stays in this repository, served only by the dev server** (`bun run dev`),
  until the owner decides where it lives (D5). `PUBLIC_PREFIXES` keeps `/landing`, so the page
  still renders signed-out in development. The comment there (`:21-23`) and the one in
  `src/app/landing/page.tsx:24-26` say that an installed image does not contain the page, and
  why. *(planner default)*
- **D3. It is proven on an installed bundle, not assumed (`AGENTS.md` rule 7, principle 5).**
  - `release.yml`'s install proof (`:1414-1420`) also asserts that
    `curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:3000/landing` is `404`. A `200`
    fails the job.
  - `.dockerignore` joins the workflow's `pull_request.paths` (`:40-46`), so a PR that edits it
    runs the dry run and the install proof.
  - A unit test in `ops/release/test_release_workflow.py` asserts that both paths are excluded.
    That file already reads the repository's workflow; neither it nor `test_release_files.py`
    reads `.dockerignore` today.
    A deleted line then fails fast, without a full image build.
- **D4. The feature page and the backlog say where the page is.**
  - `landing-page.md` decision 1 ("Where it lives") gets a new row: not in an installed console
    (`WEB-2`). The limits say so.
  - `WEB-1-AC1` to `-AC17` stay. They are run against the dev server until D5 is decided, and
    the page says that.
  - `WEB-1` stays In Acceptance, because the owner has taken it out of the QA-1 round
    (`:166`).
- **D5. Where the landing page will live instead: open, for the product owner.** The code does
  not settle this. Options:
  1. **A static export deployed as its own site** under the RantAI public domain. The page
     calls no API (`landing-page.tsx:26`, `WEB-1-AC17`), so it can be a static build. This
     needs either a small separate Next app or a `next build` with only this route. The
     deployment target (the Cloudflare account the team already uses, or another host) is the
     owner's choice. *(planner recommendation)*
  2. **Its own repository**, with the 3D scene and copy moved there and deleted here.
  3. **A page in the existing RantAI marketing site.**

  Whichever is chosen, two things change:
  - the "Open the console" link (`landing-copy.ts:14`, `"/login"`, relative today) has no single
    console to point to from a public site. It becomes a link to the install guide, or it is
    removed (owner);
  - `three`, used only by the landing page (no other `from "three"` in `src/`), can then leave
    the console's `package.json`.

  Neither change is in this item.

## 2. Anchors (verified at `9c066426`)

- `src/app/landing/page.tsx:1-37` (route, comment `:24-26`).
- `src/features/landing/` (37 files). `landing-fonts.ts:9` (`next/font/google`),
  `landing-copy.ts:13-17` (`LINKS`), `landing-page.tsx:26` (static, no API).
- `src/features/auth/auth-provider.tsx:21-23` (comment), `:29` (`PUBLIC_PREFIXES`), `:31-33`
  (`isPublicPath`).
- `src/features/auth/auth-provider.test.ts:5-14`.
- `src/components/app-shell/app-frame.tsx:34`.
- `.dockerignore:1-52` (the header names what the build must see; the excluded block starts at
  `:41`).
- `Dockerfile.frontend:27`, `:45`.
- `.github/workflows/release.yml`:
  - `pull_request.paths` `:40-46`;
  - dry-run console build `:273-282`;
  - release console build `:869-878`;
  - install proof `:1256`, the console check `:1414-1420`.
- `.github/workflows/deploy-staging.yml:207`; `docker-compose.yml:781-798`.
- `ops/release/test_release_workflow.py`, `ops/release/test_release_files.py`.
- `docs/core/features/landing-page.md:29-31`, `:70-71`, `:87`, `:117-133`, `:148-150`,
  `:164-167`.

## 3. Tasks (one commit each, in order)

- **T1. Leave the page out of the image.**
  - Add the two `.dockerignore` lines (D1) with their comment.
  - Update the two source comments (D2).
  - Build the image locally once:
    `docker build -f Dockerfile.frontend -t lakehouse-console:web2 .`
  - Check that the route is gone, by looking at the built image and not assuming
    (rule 1): `docker run --rm --entrypoint sh lakehouse-console:web2 -c 'find .next/server/app -maxdepth 1 -name "landing*"'`
    must print nothing. Write the command and its output in the handoff.
  - Then run it with `-e RUST_API_URL=…` and curl `/landing` for a 404.
  - *Accept:* the two outputs above, `bun run typecheck`, `bun run lint`, and
    `bun test src/features/auth`. The dev build still has the page, and its test stays.
- **T2. The proof in the release.**
  - The `/landing` → `404` assertion in the install proof (D3).
  - `.dockerignore` in `pull_request.paths`.
  - The unit test that reads `.dockerignore` (D3). If `test_release_workflow.py` pins the list of
    `paths`, it is updated to match, with the reason.
  - *Accept:* `python3 -m pytest ops/release -q`. The PR's own `Release` dry run and install
    proof must be green on the PR; quote their run ids in the handoff.
- **T3. The pages.** Planner or developer; docs only.
  - `landing-page.md`: decision row 6, the limits line, the AC note (D4), and open question 2
    replaced by D5.
  - `BACKLOG.md` row `WEB-2`.
  - *Accept:* `git diff --stat` touches only `docs/`.

Full block before handoff: the TypeScript line, `python3 -m pytest ops/release -q`, and the
PR's release dry run (rule 8: proven by a real build and install, not by `config`).

## 4. Out of scope

- Building or deploying the landing page's new home (D5), and moving `three` out of
  `package.json`.
- Changing the "Open the console" link.
- Removing `/landing` from `PUBLIC_PREFIXES`. It stays for the dev server. On an install it only
  means the 404 shows without the chrome.
- A console run from source outside Docker still serves the page. `README.md:174-180` lists
  `bun run build` then `bun start` as the production build of the source tree, but an install is
  the signed bundle (`lakehousectl install`, the release's install proof) or compose, and both
  use the image. This is written in the feature page's limits. Whether `README.md` should say
  that the source build includes the page is a doc line in T3.

## 5. Handoff (developer appends below)

## 6. Review (planner appends below)

---

### Proposed BACKLOG.md row

| `WEB-2` | Task | An installed Lakehouse serves the landing page: `/landing` is a public route of the console app (`src/app/landing`, `PUBLIC_PREFIXES`), and every console image (release, staging, compose) builds it in. Product owner decided 2026-11-02: it is a separate product page, not available on an install. Leave `src/app/landing` and `src/features/landing` out of the console image (`.dockerignore`), and prove `/landing` is a 404 in the release's install proof. Where the page lives instead is the owner's decision | Business | P2 | Todo |  | `features/landing-page.md` QA-1 decisions, owner 2026-11-02 |
