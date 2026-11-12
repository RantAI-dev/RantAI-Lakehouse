# RantAI Lakehouse site

The public product page of RantAI Lakehouse, in English (`/`) and Indonesian
(`/id/`). It is a Next.js app built as a **static export**: `next build`
writes plain files to `out/`, and any static file host serves them. There is
no server, no API call, no cookie and no analytics.

It is its own package (`WEB-5`, decision D1): its own `package.json` and
`bun.lock`, not a member of the console's workspace, and the console's checks
do not read it. The plan is
`docs/superpowers/plans/2026-11-04-web-5-lakehouse-site.md`, the targets are in
`docs/core/specs/web-5.md`, and the feature page is
`docs/core/features/landing-page.md`.

## Build

`NEXT_PUBLIC_SITE_URL` is required. It is the absolute `https://` origin the
site will be served from, with no path. Every canonical link, hreflang link,
Open Graph URL, the sitemap and the robots file are built from it, so the
build refuses to run without it, or with an `http://` value or a path, and
there is no default.

```bash
cd site
bun install --frozen-lockfile
NEXT_PUBLIC_SITE_URL=https://lakehouse.example.com bun run build
```

The result is `site/out/`. Upload that folder as it is.

## Serve `out/`

Any static host works, as long as it serves `index.html` for a directory URL
(`/id/` is `out/id/index.html`) and `404.html` for a missing path. Every
common host does both by default. To look at a build locally:

```bash
npx serve -l 3400 out
```

## Develop

```bash
NEXT_PUBLIC_SITE_URL=https://example.invalid bun run dev   # http://localhost:3400
```

## Checks

```bash
bun run lint && bun run typecheck && bun run test && bun run build
```

CI runs the same steps in the `site` job of `.github/workflows/ci.yml`
whenever a file under `site/` changes, and on every push to `main`.

## Rules

- **Text lives in messages.** `src/messages/<locale>/<namespace>.json`, the
  same keys in both languages; a test fails on a missing, extra or empty
  key. Brand and product names are not translated.
- **Claims trace to the backlog.** The page claims only what
  `docs/core/BACKLOG.md` marks Released, Done or In Acceptance. No logos,
  numbers, testimonials, prices or "trusted by".
- **Never run prettier** (repository rule); match the file's style by hand.
