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
