# Committed fonts

These are the dashboard and landing-page faces the console loads through
`next/font/local` (OPS-16, decision D1), so they need no network at build or at
runtime: `src/app/fonts.ts` (dashboard fonts) and
`src/features/landing/landing-fonts.ts` (the landing page's display face). The
ids, weights and CSS variables are the ones `next/font`'s Google loader
produced before, so `lakehouse_bi::formatting::SHIPPED_FONTS` and
`lib/dashboard-look.ts` need no change.

The root layout's Geist and Geist Mono are **not** here yet: they still come
from `next/font`'s Google loader in `design-system/fonts/fonts.ts`. An offline
build therefore still fails on those two faces; OPS-16's T1 is blocked on that
and the plan's Developer report records it.

Before this, `next/font`'s Google loader fetched at build time; when that fetch
failed the build failed with `Module not found: Can't resolve
'@vercel/turbopack-next/internal/font/google/font'` (CI runs `41446159405`,
`41449722905`, `41480029356` on 2026-11-02).

## Files

Each file is the `latin` block of the Google Fonts CSS 2 request below,
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
| Instrument Serif | `instrument-serif/InstrumentSerif-Italic-latin.woff2` | <https://fonts.gstatic.com/s/instrumentserif/v5/jizHRFtNs2ka5fXjeivQ4LroWlx-6zAjjH7M.woff2> | `v5` | static, italic 400 |

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
- **Instrument Serif**: the landing page uses italic 400 only, and that is a
  static face.

## Licenses

`OFL.txt` in each directory is the upstream license file from Google Fonts'
copy of the family (<https://github.com/google/fonts/tree/main/ofl>): `inter`,
`ibmplexsans`, `sourceserif4`, `jetbrainsmono`, `instrumentserif`.
