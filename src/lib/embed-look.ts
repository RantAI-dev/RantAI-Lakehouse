/**
 * The look of an embed, set in the embed address, not the token (`BI-26` plan
 * D6, feature decision 3): `theme=light|dark|dashboard`, `logo=none`,
 * `background=transparent`. None of it is secret, so the host page can change
 * it without signing a new token. An unknown value is ignored.
 */

export type EmbedTheme = "light" | "dark" | "dashboard"

export type EmbedLook = {
  /** `dashboard` keeps the dashboard's own theme. */
  theme: EmbedTheme
  /** Whether the name shows — the workspace's own, or the product's when none is set (`ADM-2`). Anyone may hide it (feature decision 4). */
  showName: boolean
  /** Let the host page's background show through. */
  transparent: boolean
}

export const DEFAULT_EMBED_LOOK: EmbedLook = { theme: "dashboard", showName: true, transparent: false }

/**
 * The class `<html>` carries while an embed is transparent (`BI-64`). Its rules
 * live in `src/app/globals.css` only; the root layout puts it on the server
 * render from {@link EMBED_TRANSPARENT_HEADER}, and `EmbedView` keeps it right
 * in the browser (`PR #648 review 1`).
 */
export const EMBED_TRANSPARENT_CLASS = "embed-transparent"

/**
 * The request header the proxy sets for an embed whose address asks
 * `background=transparent`, so the root layout can put
 * {@link EMBED_TRANSPARENT_CLASS} on `<html>` before the first paint instead of
 * after hydration (`PR #648 review 1`).
 */
export const EMBED_TRANSPARENT_HEADER = "x-embed-transparent"

type Params = Record<string, string | string[] | undefined>

function one(params: Params, key: string): string | undefined {
  const v = params[key]
  return Array.isArray(v) ? v[0] : v
}

/** The look in an embed address's query. */
export function parseEmbedLook(params: Params | undefined): EmbedLook {
  if (!params) return DEFAULT_EMBED_LOOK
  const theme = one(params, "theme")
  return {
    theme: theme === "light" || theme === "dark" || theme === "dashboard" ? theme : "dashboard",
    showName: one(params, "logo") !== "none",
    transparent: one(params, "background") === "transparent",
  }
}

/** The dashboard's theme with the address's choice laid over it (`light` and `dark` win). */
export function themeFor(boardTheme: string | undefined, look: EmbedLook): string | undefined {
  return look.theme === "dashboard" ? boardTheme : look.theme
}
