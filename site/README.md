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

## Security headers and caching (WEB-5 D18)

The build already carries what a page can say about itself, in every
HTML file's `<head>`:

- a **Content Security Policy** `<meta http-equiv>`, first in `<head>`
  (`src/lib/csp.ts`, written by `scripts/csp-meta.ts` as the last step of
  `bun run build`): everything from this origin only, no third-party host;
- `<meta name="referrer" content="strict-origin-when-cross-origin">`.

A meta tag cannot set the rest, so the host must send them. The hosting is
not chosen yet (D2), so no host file is in the repository; when it is,
paste the block for that host. All of them send the same headers:

| Header | Value | Why |
| --- | --- | --- |
| `Strict-Transport-Security` | `max-age=63072000; includeSubDomains` | HTTPS only, for two years (only once the domain serves HTTPS) |
| `X-Content-Type-Options` | `nosniff` | a file is only what its type says |
| `X-Frame-Options` | `DENY` | the site cannot be framed (with `frame-ancestors 'none'` below) |
| `Content-Security-Policy` | `frame-ancestors 'none'` | the framing rule a meta tag ignores; the rest of the policy is in the page |
| `Referrer-Policy` | `strict-origin-when-cross-origin` | the same as the meta tag, for files that are not HTML |
| `Permissions-Policy` | `camera=(), microphone=(), geolocation=()` | the site uses none of them |

Caching: `/_next/static/*` file names carry a content hash, so they can be
cached for a year and marked immutable. Every other file keeps its name
between builds (the HTML, `og.png`, the icons, the sitemap), so it is
revalidated: `public, max-age=0, must-revalidate`.

**Vercel** (`vercel.json` next to `out/`, with the output directory set to
`out`):

```json
{
  "headers": [
    {
      "source": "/(.*)",
      "headers": [
        { "key": "Strict-Transport-Security", "value": "max-age=63072000; includeSubDomains" },
        { "key": "X-Content-Type-Options", "value": "nosniff" },
        { "key": "X-Frame-Options", "value": "DENY" },
        { "key": "Content-Security-Policy", "value": "frame-ancestors 'none'" },
        { "key": "Referrer-Policy", "value": "strict-origin-when-cross-origin" },
        { "key": "Permissions-Policy", "value": "camera=(), microphone=(), geolocation=()" },
        { "key": "Cache-Control", "value": "public, max-age=0, must-revalidate" }
      ]
    },
    {
      "source": "/_next/static/(.*)",
      "headers": [{ "key": "Cache-Control", "value": "public, max-age=31536000, immutable" }]
    }
  ]
}
```

**Cloudflare Pages or Netlify** (a `_headers` file copied into `out/`
before upload):

```text
/*
  Strict-Transport-Security: max-age=63072000; includeSubDomains
  X-Content-Type-Options: nosniff
  X-Frame-Options: DENY
  Content-Security-Policy: frame-ancestors 'none'
  Referrer-Policy: strict-origin-when-cross-origin
  Permissions-Policy: camera=(), microphone=(), geolocation=()
  Cache-Control: public, max-age=0, must-revalidate

/_next/static/*
  Cache-Control: public, max-age=31536000, immutable
```

Netlify applies every matching block, and Cloudflare Pages joins repeated
headers, so on both the static files would carry two `Cache-Control`
values. On those hosts, move the `Cache-Control` line out of `/*` into a
block per kind of file (`/*.html`, `/og.png`, `/sitemap.xml`…), or check
the response with `curl -sI` after the first deploy.

**nginx** (inside the `server` block that serves `out/`):

```nginx
root /srv/lakehouse-site/out;
error_page 404 /404.html;

add_header Strict-Transport-Security "max-age=63072000; includeSubDomains" always;
add_header X-Content-Type-Options "nosniff" always;
add_header X-Frame-Options "DENY" always;
add_header Content-Security-Policy "frame-ancestors 'none'" always;
add_header Referrer-Policy "strict-origin-when-cross-origin" always;
add_header Permissions-Policy "camera=(), microphone=(), geolocation=()" always;

location /_next/static/ {
  # add_header in a location replaces the server's list, so repeat them.
  add_header Strict-Transport-Security "max-age=63072000; includeSubDomains" always;
  add_header X-Content-Type-Options "nosniff" always;
  add_header X-Frame-Options "DENY" always;
  add_header Content-Security-Policy "frame-ancestors 'none'" always;
  add_header Referrer-Policy "strict-origin-when-cross-origin" always;
  add_header Permissions-Policy "camera=(), microphone=(), geolocation=()" always;
  add_header Cache-Control "public, max-age=31536000, immutable" always;
}

location / {
  add_header Strict-Transport-Security "max-age=63072000; includeSubDomains" always;
  add_header X-Content-Type-Options "nosniff" always;
  add_header X-Frame-Options "DENY" always;
  add_header Content-Security-Policy "frame-ancestors 'none'" always;
  add_header Referrer-Policy "strict-origin-when-cross-origin" always;
  add_header Permissions-Policy "camera=(), microphone=(), geolocation=()" always;
  add_header Cache-Control "public, max-age=0, must-revalidate" always;
  try_files $uri $uri/ =404;
}
```

The 404: one `404.html` answers every missing path, in English and then
in Indonesian, because some hosts serve only the root one. Vercel,
Netlify and Cloudflare Pages use it by default; nginx needs the
`error_page` line above.

The short links rantai.dev has (`/github`, `/linkedin`…) are host
redirects. They wait for the host (D17); a static export cannot redirect.

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

- **No third-party requests.** The policy allows this origin only; a new
  font, script or image host means changing `src/lib/csp.ts` on purpose.

- **Text lives in messages.** `src/messages/<locale>/<namespace>.json`, the
  same keys in both languages; a test fails on a missing, extra or empty
  key. Brand and product names are not translated.
- **Claims trace to the backlog.** The page claims only what
  `docs/core/BACKLOG.md` marks Released, Done or In Acceptance. No logos,
  numbers, testimonials, prices or "trusted by".
- **Never run prettier** (repository rule); match the file's style by hand.
