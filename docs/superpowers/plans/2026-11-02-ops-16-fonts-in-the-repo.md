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

## 5. Review (planner appends below)
