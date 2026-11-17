# OPS-16: the console's fonts live in the repository, so a build never downloads them

- **Planner:** Claude Opus, 2026-11-02. **Base:** `main` at `9ad7231a` or later.
- **Branch:** `fix/ops-16-fonts-in-the-repo`.

## 0. What is wrong, with evidence

- The console build fetches its fonts from Google at BUILD time, through `next/font/google`. That happens in `src/app/fonts.ts` (Inter, IBM Plex Sans, Source Serif 4, JetBrains Mono) and `src/features/landing/landing-fonts.ts` (Instrument Serif).
- When the fetch fails, the build fails with `Module not found: Can't resolve '@vercel/turbopack-next/internal/font/google/font'`. On 2026-11-02 alone that broke three CI runs:
  - #346, run `41446159405`;
  - #344's release dry run, run `41449722905`;
  - #354, run `41480029356`.
- A build behind a firewall or offline cannot build the console at all. A release built from a mirror or an air-gapped build host would fail the same way.

## 1. Decisions

- **D1.** The five font families are committed as `woff2` files (latin subset, the same weights and styles as today) under `src/app/fonts/`. They load through `next/font/local`, with the SAME `variable` names, `display: "swap"` and `preload: false` (where today's code sets it).
  - Nothing else in the console changes. `dashboard-look.ts` and `lakehouse_bi::formatting::SHIPPED_FONTS` keep their ids.
- **D2.** The files come from the fonts' upstream releases, or from Google Fonts' own download of the same version. Each family's `OFL.txt` (SIL Open Font License 1.1) sits beside it, which the license requires. Name the source and version of each file in a `src/app/fonts/README.md`.
- **D3.** No `next/font/google` import remains anywhere in `src/`. A test or lint fails if one comes back (`git grep -n "next/font/google" -- src` must be empty, in a vitest that reads the files).
- **D4.** No variable font axes are added and no weight is dropped. A weight that the current code asks for and the upstream release lacks as a static file is used from the variable file instead, and the README says so.

## 2. Tasks

1. **T1. The files and the loaders.**
   - Add the `woff2` files, the `OFL.txt` files and the README.
   - Rewrite both loaders with `next/font/local`.
   - Add the D3 test.
   - *Accept:*
     - `bun run typecheck`, `bun run lint`, and `bun test src/app src/features/landing`;
     - `bun run build`, with the network to `fonts.googleapis.com` and `fonts.gstatic.com` blocked. On Linux, add `0.0.0.0 fonts.googleapis.com` and `0.0.0.0 fonts.gstatic.com` to `/etc/hosts` with sudo, or run the build in a container started with `--network none` and the dependencies already installed.
     - Quote the build's last lines in the handoff. Restore `/etc/hosts` afterwards.
2. **T2. The docs.** Add a `CHANGELOG.md` line under `[Unreleased]`, and a sentence in `docs/CI.md` where it lists the flaky checks, if it lists this one.

## 3. Out of scope

Changing which fonts are offered, changing the root layout's Geist fonts (they come from the `geist` package, not from Google), and any visual change.

## 4. Handoff

### Developer report

**T1's offline acceptance does not fit the code.** §3 says the root layout's
Geist fonts "come from the `geist` package, not from Google" and are out of
scope. At `9ad7231a` they do not:

- `src/app/layout.tsx:3` imports `geist, geistMono` from
  `@rantai/design-system/fonts/fonts`;
- `tsconfig.json:23` maps `@rantai/design-system` to `./design-system`, so that
  is `design-system/fonts/fonts.ts:22`:
  `import { Geist, Geist_Mono } from "next/font/google"`;
- neither `package.json` nor `design-system/package.json` has a `geist`
  dependency.

So the build still fetches Geist and Geist Mono at build time. The five faces
this plan moves are fine, but T1's "*`bun run build`, with the network to
`fonts.googleapis.com` and `fonts.gstatic.com` blocked*" cannot pass without
editing `design-system/fonts/fonts.ts`, which §3 forbids. D3's grep covers
`src/` only, so it does not catch that file either.

**Evidence.** `bun install` installed the dependencies; `.next` was removed
first so no cached font could hide a fetch. The build ran in a container with
no network (plan §2 T1 allows this):

```bash
rm -rf .next
docker run --rm --network none --user "$(id -u):$(id -g)" \
  -e HOME=/tmp -e NEXT_TELEMETRY_DISABLED=1 \
  -v "$PWD:$PWD" -w "$PWD" oven/bun:1 bun --bun next build
```

Exit 1. Turbopack reported exactly two errors, both Geist, both reached from
`./src/app/layout.tsx`; nothing named the five committed families:

```text
> Build error occurred
Error: Turbopack build failed with 2 errors:
[next]/internal/font/google/geist_e763f05b.module.css
Error: next/font: error:
Failed to fetch Geist from Google Fonts.
...
[next]/internal/font/google/geist_mono_1732b1c5.module.css
Error: next/font: error:
Failed to fetch Geist Mono from Google Fonts.
...
error: "next" exited with code 1
```

**Not done.** T2 (a `CHANGELOG.md` line and `docs/CI.md`) was not started: it
documents T1, and T1 is not accepted.

**Decision needed from the planner** (the developer did not improvise or widen
the scope):

1. Extend T1 to commit Geist and Geist Mono latin `woff2` under
   `src/app/fonts/geist/` with the same README recipe, and point
   `design-system/fonts/fonts.ts` at `next/font/local` while keeping
   `--font-sans` / `--font-mono` and today's preload behaviour. Widen D3's
   guard to `design-system/` too. This is the D1/D2 shape.
2. Or add the `geist` npm package and import `geist/font/sans` and
   `geist/font/mono`, which ship their own files and use `next/font/local`.
   That adds a dependency, so it is the planner's call.
3. Or narrow T1's acceptance so the two Geist faces may still be fetched,
   which leaves OPS-16's failure mode in place for every page that renders the
   root layout.

### Handoff (blocked; no PR opened)

- Commits on `fix/ops-16-fonts-in-the-repo`:
  - `2e4eeba0` feat(ops-16): commit the console's five fonts and load them
    locally -- 14 files: five `woff2`, five `OFL.txt`, `src/app/fonts/README.md`,
    `src/app/fonts.test.ts`, and the two loaders.
  - the commit that carries this report.
- Ran before committing `2e4eeba0`, in the foreground, on its exact content:
  - `bun run typecheck` -> exit 0.
  - `bun run lint` -> exit 0, 0 errors, 10 warnings (all pre-existing, none in
    a file this commit touches).
  - `bun test src/app src/features/landing` -> 20 pass, 0 fail, 3 files.
  - `grep -rn "next/font/google" src` -> no match.
  - `file src/app/fonts/*/*.woff2` -> all five are `Web Open Font Format
    (Version 2)`.
  - offline `bun --bun next build` in `--network none` -> exit 1; its last
    lines are quoted in the Developer report above.
- Not verified: the offline build (T1's acceptance), because of the Geist
  loader above. T2 was not started. No PR was opened, and nothing was
  force-pushed.

### Handoff (T1 part 2 and T2 complete, 2026-11-02)

**Commits** on `fix/ops-16-fonts-in-the-repo`, pushed after each; never
force-pushed, no PR opened:

- `2e4eeba0` feat(ops-16): commit the console's five fonts and load them
  locally — unchanged, 14 files (five `woff2`, five `OFL.txt`,
  `src/app/fonts/README.md`, `src/app/fonts.test.ts`, the two loaders).
- `560621d5` feat(ops-16): load Geist from committed files (D5) — 8 files:
  `design-system/fonts/geist/Geist-wght.woff2` and `OFL.txt`,
  `design-system/fonts/geist-mono/GeistMono-wght.woff2` and `OFL.txt`,
  `design-system/fonts/fonts.ts`, `design-system/README.md`,
  `src/app/fonts.test.ts`, `src/app/fonts/README.md`.
- `b4fca976` docs(changelog): OPS-16, the fonts are committed and load
  locally (D5) — 1 file, 1 insertion.
- the commit that carries this handoff — this plan only.

**D5, done.** Geist and Geist Mono are vercel/geist-font release `v1.9.0`
(`geist-font-v1.9.0.zip`, retrieved with `curl -fL --max-time 120` on
2026-11-02), byte for byte: `Geist/webfonts/Geist[wght].woff2` and
`GeistMono/webfonts/GeistMono[wght].woff2`. `file` reports `Web Open Font
Format (Version 2)` for both. SHA-256 `5c5588…14cae512` (Geist, 69,576
bytes) and `cf5bf5…45b46b25` (Geist Mono, 71,272 bytes) are in the README.
Each directory has the release's root `OFL.txt` (SIL OFL 1.1, "Copyright
2024 The Geist Project Authors").

**Where, and why.** The files are in `design-system/fonts/geist/` and
`design-system/fonts/geist-mono/`, the exception D5 allows, not D5's
default `src/app/fonts/geist/`. `design-system/README.md` documents the
folder as plug-and-play, ready to copy into another Next.js repo; a module
inside it cannot reach into `src/`, so the fonts that
`design-system/fonts/fonts.ts` loads must travel with it. Both READMEs say
so, and `src/app/fonts/README.md` carries the source, version and hashes.

**The loader.** `design-system/fonts/fonts.ts` uses `next/font/local` with
the same `--font-sans` / `--font-mono` variables, `display: "swap"` and
`preload: true` as the Google loader had (the five D1 loaders keep their
`preload: false`). It declares `weight: "100 900"`, `style: "normal"`; the
Google loader's `subsets: ["latin"]` has no local equivalent and is gone.

**One deviation, stated.** D5 says "latin `woff2` files". The release ships
one webfont per face covering the full character set (728 `cmap` entries
for Geist, 889 for Geist Mono) and no latin subset, so the committed files
are the release's full variable fonts — a superset of the Google `latin`
subset the old loader downloaded. Sourcing the latin subset from Google
instead would have broken D5's "from vercel/geist-font's release". The
README records this.

**The guard (D3, widened).** `src/app/fonts.test.ts` now scans `src/` and
`design-system/`, and asserts each root exists so a renamed tree cannot
pass by scanning nothing. A temporary `design-system/__guard_probe.ts`
holding the banned module id made it fail (0 pass, 1 fail) and was removed;
no probe file is in any commit.

**Commands and counts** (foreground, on `560621d5`'s content; T2 changes
only `CHANGELOG.md`, so none of these needed a re-run):

- `bun install --frozen-lockfile` — 739 packages installed, exit 0.
- `bun run typecheck` — exit 0.
- `bun run lint` — exit 0, 0 errors, 10 warnings; all 10 are in 9 files this
  branch does not touch (`data-table.tsx`, `sidebar.tsx`, `alerts-page.tsx`,
  `login-page.tsx`, `use-copilot.ts`, `use-data-table.ts`,
  `dashboard-specs.ts`, `uploads.test.ts`, `http.ts`).
- `bun test src/app src/features/landing` — 20 pass, 0 fail, 3 files.
- `bun test src/app/fonts.test.ts` — 1 pass, 0 fail.
- `grep -rn "next/font/google" src design-system` — no match.
- `file` on both `woff2` files — `Web Open Font Format (Version 2)`.
- `find design-system -name '*.test.*' -o -name '*.spec.*'` — none, so no
  design-system test folder to run.

**Offline build (T1's acceptance) — exit 0.** `.next` was removed first,
`node_modules` was already installed, and the container had no network:

```bash
rm -rf .next
docker run --rm --network none --user "$(id -u):$(id -g)" \
  -e HOME=/tmp -e NEXT_TELEMETRY_DISABLED=1 \
  -v "$PWD":/app -w /app oven/bun:1 bun --bun next build
```

Its last lines (`✓ Compiled successfully in 23.7s`):

```text
├ ƒ /render/dashboard/[token]
├ ƒ /services
├ ƒ /tables/[id]
├ ƒ /tenant-management
├ ƒ /user-management
└ ƒ /workloads


ƒ Proxy (Middleware)

ƒ  (Dynamic)  server-rendered on demand
```

`.next` was removed again after the run; nothing generated is staged.

**T2.** One line under `[Unreleased]` → `Fixed` in `CHANGELOG.md`, naming
the backlog, plan, decisions D1/D4/D5, the three CI failures, the measured
offline build and the two limits (Geist's full charset, its location).
`docs/CI.md` is unchanged: it lists no entry for this flake — no
`fonts.googleapis.com`, no font build failure, none of the three run ids
(`grep` found no match) — so there was no sentence to add.

**Not verified / not done.**

- The full `bun run test` over all of `src/` was not run; only the plan's
  `src/app src/features/landing` and the guard above.
- The plan's other acceptance variant, a `bun run build` with the two
  Google hosts blocked in `/etc/hosts`, was not run; the `--network none`
  container is the variant the plan allows and the one measured above.
- The full verification block was not run: this branch touches no Rust,
  Python, compose or migration file.
- Geist is not a `latin` subset, as stated above; that is the one place
  where D5's wording and the release disagree.

## 5. Review (planner appends below)

### Planner decision on the developer report (2026-11-02, at `a4e48156`)

**The developer is right.** Section 3's "Geist comes from the `geist` package" was wrong. `design-system/fonts/fonts.ts:22` imports `Geist` and `Geist_Mono` from `next/font/google`, and there is no `geist` dependency. Option 1 is chosen:

- **D5.** Geist and Geist Mono (OFL-1.1, from vercel/geist-font's release) are committed as latin `woff2` files under `src/app/fonts/geist/`, each with its `OFL.txt`. `design-system/fonts/fonts.ts` loads them through `next/font/local`, keeping the same CSS variables (`--font-sans`, `--font-mono`) and the same `preload` and `display`. The README gains their source and version.
  - The design-system file may import from `src/app/fonts/geist/` by a relative path. If the design system must stay self-contained, put the files in `design-system/fonts/geist/` instead, and say which one you chose and why.
- **D3 is widened.** The no-`next/font/google` test reads `src/` AND `design-system/`.
- No new npm dependency (option 2 is rejected: it adds a dependency to remove a download).

The T1 acceptance is unchanged: the offline build, with `--network none`, must exit 0.

### Review round 1 (planner, 2026-11-02, at `39a55019`)

- **D1/D2/D5.** All seven faces (the five dashboard and landing families, plus Geist and Geist Mono) are committed as `woff2`, 22 to 122 KB each. Each has its `OFL.txt` beside it, and the README gives each one's source, version and SHA-256. The CSS variable names are unchanged.
- **Accepted deviation:** geist-font 1.9.0 ships no latin subset, so the full-charset variable file is used, and it is recorded. Geist lives in `design-system/fonts/`, which D5 allows, so the design-system package stays self-contained.
- **D3.** `src/app/fonts.test.ts` fails on any `next/font/google` in `src/` or `design-system/`. The developer proved it fails on a probe file.
- **The point of the item:** the offline build was run with `docker run --network none … next build` after removing `.next`. It exited 0 (`✓ Compiled successfully`).
- No `docs/CI.md` change, because it lists no entry for this flake.

**Open: no BLOCKER, no SHOULD-FIX.** The merge waits for the PR's CI. The frontend job is the one this fixes.
