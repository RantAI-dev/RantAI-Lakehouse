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

## 4. Handoff (developer appends below)

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

## 5. Review (planner appends below)
