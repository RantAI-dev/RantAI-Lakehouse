import { NextResponse } from "next/server"
import type { NextRequest } from "next/server"
import { embedFramePolicy, type EmbedPageToken } from "@/lib/embed-frame"

/**
 * SEC-12: the embed pages say which sites may frame them.
 *
 * A page component cannot set a response header, and `next.config` headers
 * are static, but the allowed sites differ per dashboard and come from the
 * token the page was opened with. So this runs before `/embed/*` is served,
 * asks the API (`POST /api/embed/frame`, public, answers only an origin list
 * and the same empty list for a bad token) and sets
 * `Content-Security-Policy: frame-ancestors ...`. Nothing else in the console
 * is matched or changed here.
 *
 * Fails closed: an API that cannot be reached, answers slowly or answers
 * anything but a list gives `frame-ancestors 'none'`.
 */

/** Longer than this and the page is simply not frameable. */
const ASK_TIMEOUT_MS = 3_000

async function allowedOrigins(token: EmbedPageToken): Promise<unknown> {
  // Same target the `/api` rewrite in next.config.ts proxies to.
  const base = process.env.RUST_API_URL ?? "http://localhost:8080"
  // Not `apiFetch` (`PR #105 review SHOULD-FIX 2`): that is the browser-side
  // choke point — it attaches `X-Tenant` from `localStorage` and turns a `401`
  // into a redirect to `/login` — and neither exists in middleware, which runs
  // on the server before any page code. The `/api` rewrite in next.config.ts
  // applies to a browser navigation, not to a server-side fetch, so this asks
  // the API origin directly. No cookie is forwarded on purpose: the endpoint
  // is public and takes the embed token in the body, and a visitor with no
  // session must get the same `frame-ancestors 'none'` answer.
  const res = await fetch(`${base}/api/embed/frame`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ [token.kind]: token.value }),
    cache: "no-store",
    signal: AbortSignal.timeout(ASK_TIMEOUT_MS),
  })
  if (!res.ok) return []
  return ((await res.json()) as { origins?: unknown }).origins
}

export async function proxy(request: NextRequest) {
  const policy = await embedFramePolicy(request.nextUrl.pathname, allowedOrigins)
  const response = NextResponse.next()
  response.headers.set("Content-Security-Policy", policy)
  return response
}

export const config = { matcher: ["/embed/:path*"] }
