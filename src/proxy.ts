import { NextResponse } from "next/server"
import type { NextRequest } from "next/server"
import {
  ASK_TIMEOUT_MS,
  CSP_HEADER,
  NONCE_HEADER,
  contentSecurityPolicy,
  contentSiteReader,
  newNonce,
} from "@/lib/csp"
import { embedFrame, type EmbedPageToken } from "@/lib/embed-frame"

/**
 * The console's `Content-Security-Policy` for every page (`SEC-47`), and the
 * embed pages' `frame-ancestors` (`SEC-12`).
 *
 * One policy per request, built by `src/lib/csp.ts` with a fresh nonce: Next
 * reads the nonce back from the request-side header this sets and puts it on
 * its own inline scripts, and `'strict-dynamic'` lets those scripts load the
 * page's chunks. That only happens while a page is rendered per request, so
 * `src/app/layout.tsx` forces dynamic rendering for every route; a page
 * prerendered at build time would carry scripts without a nonce and a browser
 * would refuse all of them.
 *
 * `BI-1`: the PDF print view (`/render/*`) gets the same policy (its
 * `frame-ancestors` is `'none'`, no content site: the Gotenberg container
 * sends no cookie) plus the headers in `renderHeaders`.
 *
 * The sites `img-src` and `frame-src` allow are `BI-16` part B's content
 * sites, so a redirect from an allowed site to anywhere else is refused by
 * the browser:
 *
 * - A console page asks `GET /api/settings/reporting` with the request's own
 *   cookie (the list is one per deployment, but reading it needs a session),
 *   reuses the answer for a minute, and gets no extra site from a failure.
 * - An embed page asks `POST /api/embed/frame` instead, as it already does
 *   for `frame-ancestors`, and gets that board's content sites with it: an
 *   embed visitor has no session, and the whole deployment list is not
 *   theirs to learn (`PR #199 review M SHOULD-FIX BI-16 1`).
 *
 * Both calls fail closed: an unreachable, slow or refusing API, a refused
 * token, or an answer of the wrong shape means `frame-ancestors 'none'` and
 * no site beyond the page's own origin.
 *
 * Neither call is `apiFetch` (`PR #105 review SHOULD-FIX 2`): that is the
 * browser-side choke point — it attaches `X-Tenant` from `localStorage` and
 * turns a `401` into a redirect to `/login` — and neither exists here, where
 * the proxy runs on the server before any page code. The `/api` rewrite in
 * `next.config.ts` applies to a browser navigation, not to a server-side
 * fetch, so this asks `RUST_API_URL` directly (read at run time).
 */

/** The API this console serves; the same target `next.config.ts` rewrites `/api` to. */
function apiBase(): string {
  return process.env.RUST_API_URL ?? "http://localhost:8080"
}

/** `POST /api/embed/frame`: the page's allowed framing sites and content sites. */
async function askFrame(token: EmbedPageToken): Promise<unknown> {
  const res = await fetch(`${apiBase()}/api/embed/frame`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ [token.kind]: token.value }),
    cache: "no-store",
    signal: AbortSignal.timeout(ASK_TIMEOUT_MS),
  })
  return res.ok ? await res.json() : null
}

/**
 * `GET /api/settings/reporting` with the visitor's cookie. `contentSiteReader`
 * owns the timeout, the one-minute cache and the fail-closed answer; an
 * anonymous request has no cookie, gets `401` and no extra site.
 */
const readContentSites = contentSiteReader((cookie, signal) =>
  fetch(`${apiBase()}/api/settings/reporting`, {
    headers: cookie === "" ? undefined : { cookie },
    cache: "no-store",
    signal,
  }),
)

/** Whether `pathname` is (or is under) the PDF print view (`BI-1`). */
function isRenderPath(pathname: string): boolean {
  return pathname === "/render" || pathname.startsWith("/render/")
}

/** Whether `pathname` is (or is under) the embed pages, which get the board's own list. */
function isEmbedPath(pathname: string): boolean {
  return pathname === "/embed" || pathname.startsWith("/embed/")
}

/**
 * `BI-1`: the PDF print view (`/render/dashboard/<token>`) is opened by the
 * Gotenberg container with a 2-minute token in the address. It must not be
 * framed, cached, indexed, or leak the token in a `Referer`; the headers say
 * so. Nothing here asks the API: the page itself posts the token to
 * `POST /api/render/data`.
 */
export function renderHeaders(): Record<string, string> {
  return {
    "Cache-Control": "no-store",
    "Referrer-Policy": "no-referrer",
    "X-Robots-Tag": "noindex",
  }
}

export async function proxy(request: NextRequest) {
  const nonce = newNonce()
  const development = process.env.NODE_ENV === "development"
  const { pathname } = request.nextUrl

  let policy: string
  if (isRenderPath(pathname)) {
    // `BI-1`: the print view is opened by Gotenberg with a 2-minute token in the
    // address and no session, so there is no content-site list to read (the
    // call would only be a `401`). Same builder as every page, so SEC-47's
    // script, connect and frame rules apply; `frame-ancestors` stays `'none'`.
    policy = contentSecurityPolicy({ nonce, development })
  } else if (isEmbedPath(pathname)) {
    const frame = await embedFrame(pathname, askFrame)
    policy = contentSecurityPolicy({
      nonce,
      development,
      contentSites: frame.contentSites,
      frameAncestors: frame.origins,
    })
  } else {
    policy = contentSecurityPolicy({
      nonce,
      development,
      contentSites: await readContentSites(request.headers.get("cookie") ?? ""),
    })
  }

  const requestHeaders = new Headers(request.headers)
  // The guide's request-side copy: Next parses the nonce from the policy
  // header, and app code can read it from `x-nonce` for a `<Script>` of its
  // own (the console renders none today).
  requestHeaders.set(NONCE_HEADER, nonce)
  requestHeaders.set(CSP_HEADER, policy)
  const response = NextResponse.next({ request: { headers: requestHeaders } })
  response.headers.set(CSP_HEADER, policy)
  if (isRenderPath(pathname)) {
    for (const [name, value] of Object.entries(renderHeaders())) response.headers.set(name, value)
  }
  return response
}

/*
 * Every page, not the API's own answers (`/api` is the Rust service), not
 * Next's compiled assets, and not a prefetch — a prefetch is a payload, never
 * a document, and asking the settings API for one would double the calls per
 * navigation (Next's CSP guide recommends the same filter).
 */
export const config = {
  matcher: [
    {
      source: "/((?!api|_next/static|_next/image|favicon.ico).*)",
      missing: [
        { type: "header", key: "next-router-prefetch" },
        { type: "header", key: "purpose", value: "prefetch" },
      ],
    },
  ],
}
