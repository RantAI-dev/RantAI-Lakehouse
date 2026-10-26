/**
 * Which sites may frame an embed page (SEC-12).
 *
 * The embed pages (`/embed/signed/<jwt>`, `/embed/dashboard/<token>`) answer
 * with `Content-Security-Policy: frame-ancestors <sites>`. The sites are the
 * ones the dashboard's owner listed in the Share dialog; an empty list, an
 * unknown token, or any failure to find out is `frame-ancestors 'none'`, so a
 * failure can only ever make the page harder to frame, never easier.
 *
 * Pure functions live here so they can be tested; `src/proxy.ts` is the only
 * caller and does the one network call.
 */

/** The policy for a page nobody may frame. */
export const FRAME_ANCESTORS_NONE = "frame-ancestors 'none'"

/**
 * `https://host[:port]`, or `http://localhost[:port]`. The API validates the
 * same shape on write (`lakehouse_bi::embed_access::validate_origins`); it is
 * checked again here because this string ends up in a response header, and a
 * value that is not a plain origin must never reach one.
 */
const ORIGIN = /^(?:https:\/\/[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?(?::[0-9]{1,5})?|http:\/\/localhost(?::[0-9]{1,5})?)$/

/** At most this many sites, as the API allows. */
const MAX_ORIGINS = 20

/** The sites in `origins` that may frame an embed page, once each. */
export function frameAncestorOrigins(origins: unknown): string[] {
  if (!Array.isArray(origins)) return []
  const clean = origins
    .filter((o): o is string => typeof o === "string" && ORIGIN.test(o))
    .slice(0, MAX_ORIGINS)
  return [...new Set(clean)]
}

/** The `Content-Security-Policy` value for a list of allowed sites. */
export function frameAncestorsPolicy(origins: unknown): string {
  const clean = frameAncestorOrigins(origins)
  return clean.length === 0 ? FRAME_ANCESTORS_NONE : `frame-ancestors ${clean.join(" ")}`
}

/** The token an embed page was opened with, as the API's frame route takes it. */
export type EmbedPageToken = { kind: "jwt" | "token"; value: string }

/**
 * The token in an embed page's path, or `null` for any other path under
 * `/embed` (which then gets `frame-ancestors 'none'`).
 */
export function embedPageToken(pathname: string): EmbedPageToken | null {
  const match = /^\/embed\/(signed|dashboard)\/([^/]+)\/?$/.exec(pathname)
  if (!match) return null
  let value: string
  try {
    value = decodeURIComponent(match[2])
  } catch {
    return null
  }
  return value ? { kind: match[1] === "signed" ? "jwt" : "token", value } : null
}

/** What one embed page's policy needs from the API (`SEC-47`). */
export type EmbedFrame = {
  /** The sites allowed to frame the page; anything else frames nothing. */
  origins: string[]
  /**
   * The board's content sites (`BI-16` part B), as the API answered. Only
   * string entries are kept here; `contentSecurityPolicy` in `src/lib/csp.ts`
   * decides which of them may enter a header.
   */
  contentSites: string[]
}

/**
 * `SEC-47`: the frame origins and the content sites of the embed page at
 * `pathname`, asked from `POST /api/embed/frame` with the page's token.
 *
 * A path with no token asks nothing, and any failure — a refused token, an
 * unreachable or slow API, an answer that is not JSON — is the empty answer:
 * `frame-ancestors 'none'`, and no site for an image or a web card. A failure
 * can therefore only make the page stricter.
 */
export async function embedFrame(
  pathname: string,
  fetchFrame: (token: EmbedPageToken) => Promise<unknown>,
): Promise<EmbedFrame> {
  const empty: EmbedFrame = { origins: [], contentSites: [] }
  const token = embedPageToken(pathname)
  if (!token) return empty
  let answer: unknown
  try {
    answer = await fetchFrame(token)
  } catch {
    return empty
  }
  if (typeof answer !== "object" || answer === null) return empty
  const body = answer as { origins?: unknown; contentSites?: unknown }
  return {
    origins: frameAncestorOrigins(body.origins),
    contentSites: Array.isArray(body.contentSites)
      ? body.contentSites.filter((site): site is string => typeof site === "string")
      : [],
  }
}
