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

### Handoff (developer, 2026-11-02)

Branch `fix/web-2-no-landing-on-installs`, base `main` at `9c066426` (the plan commit
`3e9c1f71` on top). An installed Lakehouse does not serve `/landing`, and the release's install
proof says so.

**Commits** (all pushed, one per task; `git log --oneline origin/fix/web-2-no-landing-on-installs`):

- `ca11943e` — feat(web-2): keep the landing page out of the console image (T1, plan D1–D2).
  `.dockerignore` gains `src/app/landing` and `src/features/landing` with the reason (the owner
  decided on 2026-11-02 that the page is a separate product page and an install must not offer
  it). `src/app/landing/page.tsx` and `src/features/auth/auth-provider.tsx` say, where a reader
  looks, that the dev server serves the page and an install does not.
- `e10ae4d4` — feat(web-2): prove on the install that /landing is not served (T2, plan D3).
  The release workflow's `pull_request.paths` gains `.dockerignore`, so a PR that edits the console
  image's build context runs the dry run and the proof. The install proof asks the installed
  console for `/landing`'s status and fails the job unless it is 404. `ops/release/test_release_workflow.py`
  reads `.dockerignore` and asserts both of the landing page's folders are out of the build
  context, with Docker's own rule (last match wins, `!` re-includes), and asserts the proof asks
  for the status, names 404 and fails the job. Each of the three checks also runs against a
  synthetic case, so a check that cannot fail is itself a failure.
- `3ac4c72c` — docs(web-2): the pages say where the landing page lives (T3, plan D4–D5).
  `landing-page.md` gets decision row 6, the limits line, the AC note, and open question 2
  replaced by D5's question. `BACKLOG.md` gets the `WEB-2` row. `docs/CI.md` names the paths that
  make the release workflow run, and `.dockerignore` is one of them now (T2), so the table lists
  it. `git diff --stat` for this commit touches only `docs/`.
- A last commit adds this handoff to the plan file and nothing else; its hash is the branch head.

**T2's paths list.** The plan asks that a pinned list of `paths` be updated with the reason.
`ops/release/test_release_workflow.py` does not pin the list: it reads the workflow's own
`pull_request.paths` and asserts `.dockerignore` is among them, so no list had to be updated
there. The one place that pins the list in prose is `docs/CI.md`, and the T3 commit updates it —
the reason is in the workflow's comment next to the new entry: `.dockerignore` decides what the
console build sees, so a PR that edits it changes the image the dry run builds and the install
proof checks.

**The T1 outputs the plan asks for** (`docker build -f Dockerfile.frontend -t lakehouse-console:web2 .`
then the two checks; the build was run from this branch's tree, exit 0, one pre-existing warning
`UndefinedVar: Usage of undefined variable '$NEXT_PUBLIC_SSO_ENABLED' (line 31)`):

```
$ docker run --rm --entrypoint sh lakehouse-console:web2 -c 'find .next/server/app -maxdepth 1 -name "landing*"'
(no output — the route is not in the image)
```

```
$ docker run -d --name web2-console-check -p 3112:3000 -e RUST_API_URL=http://127.0.0.1:8080 lakehouse-console:web2
$ curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:3112/landing
404
$ curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:3112/
200
```

`/landingx` and `/landing/anything` also answer 404. The container was removed after the check
(`docker ps -a` is empty).

The dev build still has the page, and its test stays: `bun run build` from the source tree
succeeds and `.next/server/app/landing` is in it, and `bun test src/features/landing` is 19 pass
/ 0 fail.

**The proof's check, exercised.** The install proof's shell was run against a stub server with the
step's own comparison (`if [ "$landing_status" != "404" ]`): a 404 passes, a 200 prints
`an install must not serve the landing page (WEB-2): got 200` and exits 1.

**Commands run, with counts**, on the branch head, in the foreground:

- `python3 -m pytest ops/release -q` — 194 passed, 1 warning.
- `bun run typecheck` — clean.
- `bun run lint` — 649 errors, 11707 warnings, all of them in `dagster/.venv/`, an untracked local
  virtualenv that eslint walks because it is not in `.gitignore`'s ignore set for eslint. On the
  tracked files (`git ls-files '*.ts' '*.tsx' '*.js' '*.mjs' | xargs bun --bun eslint`) it is 10
  problems, 0 errors, 10 warnings, all pre-existing and none in a file this branch touched.
  **Not verified**: a green `bun run lint` on this machine — the venv's own files are what fail,
  and the PR does not touch them. CI, which has no `dagster/.venv`, is the check that matters.
- `bun test src/features/auth` — 10 pass, 0 fail, 10 `expect()` calls, 3 files.
- `bun test src` — 3136 pass, 1 skip, 0 fail, 8542 `expect()` calls, 320 files.
- `bun test src/features/landing` — 19 pass, 0 fail, 2 files.
- `python3 ops/lint/check_intra_package_imports.py` — `OK: all intra-package dispar_orchestrate
  imports resolve`.
- `python3 ops/lint/check_bare_iceberg_count.py` — `R11 guard: no bare count() against a Bronze
  Iceberg table found.`
- `(cd dagster && .venv/bin/python -m pytest dispar_orchestrate -q)` — 2411 passed, 1 skipped, 298
  warnings, 40 subtests passed in 1039.86s. (The `dlt` import needs the venv's interpreter; the
  system `python3` cannot import it. This branch touches no dagster code.)
- `docker compose --profile '*' config --quiet` — exit 0, with throwaway
  `POSTGRES_PASSWORD`/`RUSTFS_ACCESS_KEY`/`RUSTFS_SECRET_KEY`/`LAKEKEEPER_ENCRYPTION_KEY` in the
  environment (SEC-18: compose has no default credentials).
- `cd rust && cargo fmt --check` — clean.
- `cd rust && CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target cargo clippy --workspace --all-targets
  --all-features -- -D warnings` — exit 0, no warnings and no errors (`Finished dev profile in
  4m 28s` on the first run; the second run, from the warm cache, exited 0 in 1.06s).

**Not verified**

- The PR's own `Release` dry run and install proof must be green on the PR (the plan's T2 accept).
  The PR is not open yet, so there are no run ids to quote: the dry run runs once the planner
  opens the PR, and its run id goes in the review.
- `cargo test --workspace`, **not verified**: it did not finish inside the session on this
  machine. This branch touches no Rust code (`git diff origin/main...HEAD -- rust/` is empty), so a
  Rust test cannot have broken because of it. The reviewer runs it.
- A green `bun run lint` on this machine, for the reason above.

**Notes for the reviewer**

- The `WEB-2` backlog row is `In Progress` with an empty PR column, because the branch is open
  and not merged; `AGENTS.md` step 6 has the reviewer set the Status and PR after the merge.
- The `docs/core/PRODUCT.md` section 2 was not touched: it does not mention the landing page
  (`grep -rn -i landing docs/core/PRODUCT.md` is empty), so there was nothing there to correct.
- `docs/core/HANDOFF.md` was not touched either, for the same reason.
- `README.md:174-180` still lists `bun run build` then `bun start` as the production build of the
  source tree. That build still serves the page — the console image is what an install runs — and
  the plan puts this fact in the feature page's limits, which the T3 commit does. Whether
  `README.md` should also say it is the plan's open question, and it is a one-line doc change the
  planner may want before the merge.

## 6. Review (planner appends below)

---

### Proposed BACKLOG.md row

| `WEB-2` | Task | An installed Lakehouse serves the landing page: `/landing` is a public route of the console app (`src/app/landing`, `PUBLIC_PREFIXES`), and every console image (release, staging, compose) builds it in. Product owner decided 2026-11-02: it is a separate product page, not available on an install. Leave `src/app/landing` and `src/features/landing` out of the console image (`.dockerignore`), and prove `/landing` is a 404 in the release's install proof. Where the page lives instead is the owner's decision | Business | P2 | Todo |  | `features/landing-page.md` QA-1 decisions, owner 2026-11-02 |

### Review round 1 (planner, 2026-11-02, at `788cf49d`)

- **T1.** The two `.dockerignore` lines carry their why-comment, and the source comments say where the page is served. Nothing outside the two folders imports them (`git grep` checked). The developer built the image: `find .next/server/app -maxdepth 1 -name "landing*"` printed nothing, `/landing` answered 404 and `/` answered 200.
- **T2.**
  - `.dockerignore` is in the release workflow's `pull_request.paths`, so this PR runs the dry run itself.
  - The install proof fails on anything but 404.
  - The unit test reads `.dockerignore` rather than pinning a copy. That is the right choice, since the file is the one owner of the list (rule 4).
- **T3** is docs only.
- **Counts quoted in the handoff:** ops/release 194, console 3136 pass, dagster 2411, clippy on the workspace exit 0. `cargo test --workspace` is *not verified*, and no Rust changed.

**Open: no BLOCKER, no SHOULD-FIX.** The merge waits for the PR's CI and its `Release · dry run` (install proof).
