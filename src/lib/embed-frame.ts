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

/** The `Content-Security-Policy` value for a list of allowed sites. */
export function frameAncestorsPolicy(origins: unknown): string {
  if (!Array.isArray(origins)) return FRAME_ANCESTORS_NONE
  const clean = origins
    .filter((o): o is string => typeof o === "string" && ORIGIN.test(o))
    .slice(0, MAX_ORIGINS)
  return clean.length === 0 ? FRAME_ANCESTORS_NONE : `frame-ancestors ${[...new Set(clean)].join(" ")}`
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

/**
 * The `Content-Security-Policy` for the embed page at `pathname`.
 * `fetchOrigins` asks the API for the page's allowed sites; anything it
 * throws, or answers that is not a list, is the same as an empty list.
 */
export async function embedFramePolicy(
  pathname: string,
  fetchOrigins: (token: EmbedPageToken) => Promise<unknown>,
): Promise<string> {
  const token = embedPageToken(pathname)
  if (!token) return FRAME_ANCESTORS_NONE
  try {
    return frameAncestorsPolicy(await fetchOrigins(token))
  } catch {
    return FRAME_ANCESTORS_NONE
  }
}
