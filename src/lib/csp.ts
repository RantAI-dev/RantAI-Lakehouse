/**
 * The console's Content-Security-Policy (`SEC-47`).
 *
 * `src/proxy.ts` builds one policy per request and sends it as
 * `Content-Security-Policy`. The point is enforcement, not a report: an
 * injected inline script, a script from another origin, an `object` or a
 * `base` hijack, and a redirect from an allowed content site to somewhere
 * else are refused by the browser. Before this the image and web cards
 * checked only the address as written against the administrator's list
 * (`siteAllowed` in `src/lib/type-b.ts`), and a redirect was followed
 * anywhere; `img-src` and `frame-src` now decide in the browser too.
 *
 * Three decisions worth knowing when reading a policy this module built:
 *
 * - `script-src` carries a fresh nonce and `'strict-dynamic'`. Next applies
 *   the nonce to its own inline bootstrap and to every chunk that bootstrap
 *   loads, so no page needs `'unsafe-inline'` for scripts. Next only does that
 *   on a request whose `Content-Security-Policy` header carries the nonce,
 *   which is why `src/app/layout.tsx` renders every route per request: a page
 *   prerendered at build time has no nonce in its scripts, and with
 *   `'strict-dynamic'` a browser would refuse all of them.
 * - `style-src 'unsafe-inline'` is a stated exception: React, Radix and
 *   ECharts set inline `style` attributes and style elements, which a nonce
 *   cannot cover. It allows styles only, never script.
 * - `next dev` needs `'unsafe-eval'` (React's debugging) and the HMR
 *   websocket; both are added only when the caller says this is development,
 *   and a unit test holds production to neither.
 *
 * Pure functions and the one cached lookup live here so they can be tested
 * without a Next server; `src/proxy.ts` is the only caller.
 */

import { frameAncestorsPolicy } from "./embed-frame"

/** The response header this module builds a value for. */
export const CSP_HEADER = "Content-Security-Policy"

/** The request header Next reads back the nonce from (see the module doc). */
export const NONCE_HEADER = "x-nonce"

/** The most sites the API stores (`MAX_CONTENT_SITES` in `embed_access.rs`). */
export const MAX_CONTENT_SITES = 50

/**
 * A fresh nonce: 16 random bytes in base64, the alphabet Next's own parser
 * accepts (`get-script-nonce-from-header` in the installed Next).
 */
export function newNonce(): string {
  const bytes = new Uint8Array(16)
  crypto.getRandomValues(bytes)
  return btoa(String.fromCharCode(...bytes))
}

/**
 * One content site in its normal form, or `null`.
 *
 * Exactly `https://host[:port]`: no wildcard, path, query, user name, `http:`
 * or whitespace. The API stores normalised values only, and this is the last
 * check before a value becomes part of a response header, so anything that is
 * not that shape — a `;`, a space, a quote — is dropped rather than repaired.
 * Mirrors `validate_content_sites` in `lakehouse-bi`'s `embed_access.rs`.
 */
export function contentSite(value: unknown): string | null {
  if (typeof value !== "string" || !value.startsWith("https://")) return null
  const rest = value.slice("https://".length)
  // Letters, digits, `.`, `-` and the `:` before the port: nothing that could
  // end a directive or start another one.
  if (rest === "" || !/^[A-Za-z0-9.:-]+$/.test(rest)) return null
  const colon = rest.indexOf(":")
  const host = (colon === -1 ? rest : rest.slice(0, colon)).toLowerCase()
  const port = colon === -1 ? null : rest.slice(colon + 1)
  const hostOk =
    host.length > 0 &&
    host.length <= 253 &&
    host.split(".").every((label) => label !== "" && !label.startsWith("-") && !label.endsWith("-"))
  if (!hostOk) return null
  if (port === null) return `https://${host}`
  if (!/^[0-9]{1,5}$/.test(port)) return null
  const number = Number(port)
  return number >= 1 && number <= 65535 ? `https://${host}:${number}` : null
}

/**
 * The sites in `values` that are acceptable, once each, normalised, in order,
 * at most [`MAX_CONTENT_SITES`]. Not a list at all is no site.
 */
export function contentSites(values: unknown): string[] {
  if (!Array.isArray(values)) return []
  const out: string[] = []
  for (const value of values) {
    const site = contentSite(value)
    if (site !== null && !out.includes(site)) out.push(site)
    if (out.length === MAX_CONTENT_SITES) break
  }
  return out
}

export type PolicyOptions = {
  /** A value from [`newNonce`]. */
  nonce: string
  /**
   * Sites image and web cards may load from (empty: `'self' data: blob:`).
   * `unknown` on purpose: this comes from an API answer, and every entry is
   * validated here before it enters the header.
   */
  contentSites?: readonly unknown[]
  /** Sites allowed to frame this page (`SEC-12`); empty means none. */
  frameAncestors?: readonly unknown[]
  /** `next dev` only; see the module doc. */
  development?: boolean
}

/**
 * The policy for one page. The directives are always the same and in the same
 * order; only the nonce, the site lists and the two development-only sources
 * differ, so a reader can diff two policies by eye.
 */
export function contentSecurityPolicy(options: PolicyOptions): string {
  const sites = contentSites(options.contentSites)
  const images = ["'self'", "data:", "blob:", ...sites].join(" ")
  const frames = sites.length === 0 ? "'none'" : sites.join(" ")
  const script = ["'self'", `'nonce-${options.nonce}'`, "'strict-dynamic'"]
  const connect = ["'self'"]
  if (options.development === true) {
    // React's debugging uses eval, and the dev server's HMR rides a websocket.
    script.push("'unsafe-eval'")
    connect.push("ws:", "wss:")
  }
  return [
    "default-src 'self'",
    `script-src ${script.join(" ")}`,
    // Stated exception: inline style attributes and style elements are not
    // something a nonce can cover; this allows styles, never script.
    "style-src 'self' 'unsafe-inline'",
    `img-src ${images}`,
    `frame-src ${frames}`,
    `connect-src ${connect.join(" ")}`,
    "font-src 'self' data:",
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
    frameAncestorsPolicy(options.frameAncestors),
  ].join("; ")
}
