# Published landing page (GitHub Pages)

The landing page served at **https://lakehouse.rantai.dev**.

## Why a separate Next project

The console is built with `output: "standalone"`, `/api` rewrites and
`headers()`, and its root layout (`src/app/layout.tsx`) is
`export const dynamic = "force-dynamic"` because it reads the request's CSP
nonce (`SEC-47`). `output: "export"` cannot coexist with any of that, so
turning it on for the whole `src/app` tree would break the console build.

This directory is therefore a minimal Next project whose only route is `/`,
and whose layout (`app/layout.tsx`) is static: no `headers()`, no
`force-dynamic`, no `AuthProvider`/`AppFrame`, no API calls. It renders the
same `LandingPage` component the console's dev route does
(`src/features/landing/landing-page.tsx`), so the two cannot drift.

`app/globals.css` imports the console's `src/app/globals.css` (which pulls the
design system's Tailwind and tokens) and adds an explicit `@source` for
`src/features/landing`, because Tailwind's automatic content detection starts
from this project directory and none of the components live here.

`public/` is a directory of symlinks to the repository's own
`public/` files (the icons, `site.webmanifest`, and the console's
`src/app/favicon.ico`): the landing build publishes only the files the page
actually uses, not the console's other public assets.

## Build

```bash
NEXT_PUBLIC_CONSOLE_URL=https://lakehouse-demo.rantai.dev/login bun run build:landing
```

`scripts/build-landing-pages.ts` runs `next build` here (under Node, `REL-8`),
copies the export to `out-landing/` at the repository root, and adds the two
files GitHub Pages needs:

- `CNAME` — the custom domain;
- `.nojekyll` — without it, Jekyll strips `_next/` because of the leading
  underscore, and every script and stylesheet 404s.

`out-landing/` is gitignored; the artifact is produced by
`.github/workflows/deploy-landing.yml` and served from GitHub Pages.

## Why `NEXT_PUBLIC_CONSOLE_URL`

The published site has no console of its own, so "Open the console" points at
the demo console. `src/features/landing/landing-copy.ts` reads the variable at
build time and falls back to the relative `/login` the console's own route
uses, which is what local development and the unit tests get.
