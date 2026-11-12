# Committed fonts

These are the fonts the console loads through `next/font/local` (OPS-16,
decisions D1 and D5), so they need no network at build or at runtime:
`src/app/fonts.ts` loads the dashboard faces and `design-system/fonts/fonts.ts` the root
layout's Geist and Geist Mono. The ids, weights and CSS variables are the ones
`next/font`'s Google loader produced before, so
`lakehouse_bi::formatting::SHIPPED_FONTS` and `lib/dashboard-look.ts` need no
change.

Before this, `next/font`'s Google loader fetched at build time; when that fetch
failed the build failed with `Module not found: Can't resolve
'@vercel/turbopack-next/internal/font/google/font'` (CI runs `41446159405`,
`41449722905`, `41480029356` on 2026-11-02).

## Geist and Geist Mono (decision D5)

`design-system/fonts/fonts.ts` used to import `Geist` and `Geist_Mono` from
`next/font`'s Google loader, which kept a build-time fetch in place for every
page that renders the root layout (Developer report, `6a5e266a`). D5 moves both
faces to committed files and `next/font/local`, keeping `--font-sans`,
`--font-mono`, `display: "swap"` and `preload: true` as they were.

**Where, and why.** D5 names `src/app/fonts/geist/` but allows
`design-system/fonts/geist/` if the design system must stay self-contained. It
must: `design-system/README.md` describes the folder as plug-and-play, "siap
copy ke repo Next.js lain" (ready to copy into another Next.js repo), and a
module there cannot reach into `src/` without breaking that. The two faces
therefore sit in `design-system/fonts/geist/` and
`design-system/fonts/geist-mono/`, next to the loader that uses them, and travel
with the design system.

**Source.** vercel/geist-font's GitHub release `v1.9.0` (`geist-font-v1.9.0.zip`,
retrieved 2026-11-02 with `curl -fL`): `Geist/webfonts/Geist[wght].woff2` ->
`geist/Geist-wght.woff2` and `GeistMono/webfonts/GeistMono[wght].woff2` ->
`geist-mono/GeistMono-wght.woff2`, byte for byte. Both are variable fonts
(`wght` 100–900) and `file` reports `Web Open Font Format (Version 2)`.

**Not a `latin` subset.** The release ships one webfont per face covering the
full character set (728 `cmap` entries for Geist, 889 for Geist Mono), and no
latin-only file; the Google Fonts `latin` subset would have come from a
different download than the one D5 names. The files are therefore the release's
full fonts, a superset of what the Google loader used. The loader declares
`weight: "100 900"` — the axis the file carries — and has no `subsets` option,
which only the Google loader has.

| Family | File | Source | Version | SHA-256 | Face |
| --- | --- | --- | --- | --- | --- |
| Geist | `design-system/fonts/geist/Geist-wght.woff2` | <https://github.com/vercel/geist-font/releases/download/v1.9.0/geist-font-v1.9.0.zip> (`Geist/webfonts/Geist[wght].woff2`) | `v1.9.0` | `5c5588027e59e74e205b9d6c7fd877b261091cee367f6bb3a8d505da14cae512` | variable, `wght` 100–900, full charset |
| Geist Mono | `design-system/fonts/geist-mono/GeistMono-wght.woff2` | same release (`GeistMono/webfonts/GeistMono[wght].woff2`) | `v1.9.0` | `cf5bf55f69e043b1ecf04b0719119b7298e94add67ebcaa709a925b945b46b25` | variable, `wght` 100–900, full charset |

Both directories carry the release's root `OFL.txt` (SIL Open Font License 1.1,
"Copyright 2024 The Geist Project Authors"), which covers both families.

## Files

The five files under `src/app/fonts/` are each the `latin` block of the Google
Fonts CSS 2 request below,
retrieved on 2026-11-02 with a Chrome user agent. That is Google Fonts' own
download of the version it served that day, so the subset and the outlines are
exactly what `next/font`'s Google loader used. The `vNN` in the URL is Google
Fonts' version of the family and is the version named here. Every file is a real
`Web Open Font Format (Version 2)` file (`file` says so), and every directory
holds the family's `OFL.txt` (SIL Open Font License 1.1) beside the font, as
the license requires.

| Family | File | Source URL | Version | Face |
| --- | --- | --- | --- | --- |
| Inter | `inter/Inter-latin-wght.woff2` | <https://fonts.gstatic.com/s/inter/v20/UcC73FwrK3iLTeHuS_nVMrMxCp50SjIa1ZL7.woff2> | `v20` | variable, `wght` 100–900 |
| IBM Plex Sans | `ibm-plex-sans/IBMPlexSans-latin-wght.woff2` | <https://fonts.gstatic.com/s/ibmplexsans/v23/zYXzKVElMYYaJe8bpLHnCwDKr932-G7dytD-Dmu1syxeKYY.woff2> | `v23` | variable, `wght` 100–700 |
| Source Serif 4 | `source-serif-4/SourceSerif4-latin-opsz-wght.woff2` | <https://fonts.gstatic.com/s/sourceserif4/v15/vEFI2_tTDB4M7-auWDN0ahZJW1gb8tc.woff2> | `v15` | variable, `opsz` 8–60, `wght` 200–900 |
| JetBrains Mono | `jetbrains-mono/JetBrainsMono-latin-wght.woff2` | <https://fonts.gstatic.com/s/jetbrainsmono/v24/tDbV2o-flEEny0FZhsfKu5WU4xD7OwE.woff2> | `v24` | variable, `wght` 100–800 |
| Instrument Serif (moved to `site/src/fonts/instrument-serif/` by WEB-5; the console no longer loads it) | `instrument-serif/InstrumentSerif-Italic-latin.woff2` | <https://fonts.gstatic.com/s/instrumentserif/v5/jizHRFtNs2ka5fXjeivQ4LroWlx-6zAjjH7M.woff2> | `v5` | static, italic 400 |

The CSS requests that produced these `latin` blocks:

- `https://fonts.googleapis.com/css2?family=Inter:wght@100..900&display=swap`
- `https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600;700&display=swap`
- `https://fonts.googleapis.com/css2?family=Source+Serif+4:opsz,wght@8..60,200..900&display=swap`
- `https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@100..800&display=swap`
- `https://fonts.googleapis.com/css2?family=Instrument+Serif:ital@1&display=swap`

## Weights (decision D4)

No weight the old code asked for is dropped, and no axis is added that the old
code did not use.

- **Inter**: `wght` 100–900, as the variable font `next/font`'s Google loader served.
- **IBM Plex Sans**: the old code asked for 400, 500, 600 and 700. Google Fonts
  has no static `latin` files for them any more: its own download returns one
  variable file (the `fvar` table says `wght` 100–700) for all four requests.
  The loader therefore declares `weight: "400 700"`, the range the code uses,
  not the file's full 100–700.
- **Source Serif 4**: `wght` 200–900, as before. The file also carries `opsz`
  8–60; the loader does not set that axis, so the browser's default
  `font-optical-sizing: auto` chooses the optical size, exactly as it did with
  the Google-served file.
- **JetBrains Mono**: `wght` 100–800, as before.
- **Instrument Serif**: the landing page (since WEB-5 the Lakehouse site,
  `site/`) uses italic 400 only, and that is a static face.
- **Geist** and **Geist Mono**: `wght` 100–900, the axis the release's variable
  files carry; the old Google loader requested the same 100–900 range.

## Licenses

For the five Google-served families, `OFL.txt` in each directory is the
upstream license file from Google Fonts' copy of the family
(<https://github.com/google/fonts/tree/main/ofl>): `inter`, `ibmplexsans`,
`sourceserif4`, `jetbrainsmono`, `instrumentserif` (its directory, with its
`OFL.txt`, is now `site/src/fonts/instrument-serif/`). The Geist and Geist Mono
directories carry the `OFL.txt` from the geist-font release, described above.
