/**
 * The workspace's own name and icon in the browser tab (`ADM-2`, plan
 * decisions 3 and 4).
 *
 * **Server-only.** This module reads `process.env.RUST_API_URL` and asks the
 * API for the public branding answer, so a client component must not import
 * it: a browser has no API URL, and the request this module makes is a
 * server-side one.
 *
 * The tab title is the workspace's name, and a page with a title of its own
 * gets that name appended by the root layout's template. The tab icon is the
 * workspace's own picture when one is set, and the product's own files when
 * none is. Nothing here throws: the route is public, it may be unreachable,
 * slow, or answer something else entirely, and a browser tab must still
 * render.
 *
 * The request is the global `fetch`, not `apiFetch` (`src/services/http.ts`):
 * `apiFetch` is the browser choke point — it attaches the tenant header from
 * `localStorage` and turns a `401` into a redirect to `/login` — and this
 * request is made on the server, before any page code runs, so it carries no
 * tenant and must never redirect.
 */

import type { Metadata } from "next"
import { cache } from "react"

import { ASK_TIMEOUT_MS } from "@/lib/csp"
import { PRODUCT_NAME } from "@/lib/product-branding"
import { brandingImageUrl, type Branding, type BrandingPicture } from "@/services"

/**
 * The product's own description. It describes the product, not the
 * workspace: the workspace owns its name and its icon, not the sentence that
 * says what the console is.
 */
const PRODUCT_DESCRIPTION = "Rantai Lake - Data platform"

/** The API this console serves; read at call time (see the module doc). */
function apiBase(): string {
  return process.env.RUST_API_URL ?? "http://localhost:8080"
}

export type BrandingReaderOptions = {
  /** The timeout, for tests; production always uses [`ASK_TIMEOUT_MS`]. */
  timeoutMs?: number
}

/** One picture, when the answer holds one. */
function isPicture(value: unknown): value is BrandingPicture {
  if (typeof value !== "object" || value === null) return false
  const picture = value as { type?: unknown; version?: unknown }
  return typeof picture.type === "string" && typeof picture.version === "string"
}

/** One picture or none, when the answer holds one. */
function isPictureOrNull(value: unknown): value is BrandingPicture | null {
  return value === null || isPicture(value)
}

/**
 * The branding answer, when it is one; `null` when it is not.
 *
 * Fail closed: an answer of another shape is no branding at all, so the
 * product's own name and icon stand in. A workspace that set a name and a
 * malformed logo loses the name with it, which is the honest reading of
 * "this is not the branding answer".
 */
function brandingFrom(body: unknown): Branding | null {
  if (typeof body !== "object" || body === null) return null
  const raw = body as Record<string, unknown>
  const workspaceName = raw.workspaceName
  const logo = raw.logo
  const logoDark = raw.logoDark
  const icon = raw.icon
  if (workspaceName !== null && typeof workspaceName !== "string") return null
  if (!isPictureOrNull(logo)) return null
  if (!isPictureOrNull(logoDark)) return null
  if (!isPictureOrNull(icon)) return null
  return { workspaceName, logo, logoDark, icon }
}

/**
 * `GET /api/branding`, or `null` when the API cannot answer.
 *
 * The URL is read at call time, not when this module loads: the same server
 * process serves the console in development and in production, and the API's
 * address is only known once the environment is (`src/proxy.ts` says the same
 * of its own call).
 */
async function fetchWorkspaceBranding(
  options: BrandingReaderOptions = {},
): Promise<Branding | null> {
  const timeoutMs = options.timeoutMs ?? ASK_TIMEOUT_MS
  let body: unknown
  try {
    const res = await fetch(`${apiBase()}/api/branding`, {
      cache: "no-store",
      signal: AbortSignal.timeout(timeoutMs),
    })
    if (!res.ok) return null
    body = await res.json()
  } catch {
    // Unreachable, too slow, or not JSON: no branding.
    return null
  }
  return brandingFrom(body)
}

/**
 * The workspace's branding for the request being rendered.
 *
 * React's `cache` keeps the two callers — the root layout's metadata and its
 * body — on one request per page. Outside a React request scope it calls
 * through, so a unit test can call it directly.
 */
export const readWorkspaceBranding = cache(fetchWorkspaceBranding)

/**
 * The root layout's metadata: the workspace's name in the tab title.
 *
 * The name is in the title twice, because the title has two jobs: `default`
 * is the tab title of a page that has none of its own, and `template` is what
 * a page that has one gets appended to it. `appleWebApp.title` is the name
 * under the iOS home-screen icon, so it follows the workspace too.
 */
export function workspaceMetadata(branding: Branding | null): Metadata {
  const name = branding?.workspaceName ? branding.workspaceName : PRODUCT_NAME
  return {
    title: { default: name, template: `%s · ${name}` },
    description: PRODUCT_DESCRIPTION,
    manifest: "/site.webmanifest",
    appleWebApp: { title: name },
  }
}

/** One `<link>` in the document's head. */
export type IconLink = {
  /** `icon` or `apple-touch-icon`. */
  rel: string
  /** Where the picture is. */
  href: string
  /** The picture's media type, when the browser should be told. */
  type?: string
  /** The colour scheme the picture is for, when there is one per scheme. */
  media?: string
}

/**
 * The tab icon links.
 *
 * A workspace that set an icon owns one picture, so there is one `<link
 * rel="icon">` and no `media`: there is nothing to choose between. A
 * workspace that set none leaves the product's own files in place, one per
 * colour scheme, exactly as they were.
 */
export function workspaceIconLinks(branding: Branding | null): IconLink[] {
  const icon = branding?.icon ?? null
  if (icon === null) {
    return [
      {
        rel: "icon",
        href: "/icon-light-32.png",
        type: "image/png",
        media: "(prefers-color-scheme: light)",
      },
      {
        rel: "icon",
        href: "/icon-dark-32.png",
        type: "image/png",
        media: "(prefers-color-scheme: dark)",
      },
      { rel: "apple-touch-icon", href: "/apple-touch-icon.png" },
    ]
  }
  const href = brandingImageUrl("icon", icon.version)
  return [
    { rel: "icon", href, type: icon.type },
    { rel: "apple-touch-icon", href },
  ]
}
