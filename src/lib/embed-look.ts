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
  /** Whether the "Rantai Lake" name shows (feature decision 4: anyone may hide it). */
  showName: boolean
  /** Let the host page's background show through. */
  transparent: boolean
}

export const DEFAULT_EMBED_LOOK: EmbedLook = { theme: "dashboard", showName: true, transparent: false }

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
