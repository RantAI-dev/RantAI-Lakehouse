import type * as React from "react"

/**
 * A dashboard's own theme and font (`BI-17` part A, D6, decision 13). The
 * dashboard container applies them over the console's theme, for its tiles and
 * the ECharts `dark` flag, everywhere it is shown: the console, a public link
 * and a signed embed.
 *
 * Dark inside a light console is the `dark` class on the container (the tokens
 * and the `dark:` variants key off an ancestor with it). Light inside a dark
 * console is `theme-light`, which `globals.css` defines with the light tokens.
 */

export const THEMES = ["auto", "light", "dark"] as const
export type DashboardTheme = (typeof THEMES)[number]

export function isTheme(v: unknown): v is DashboardTheme {
  return typeof v === "string" && (THEMES as readonly string[]).includes(v)
}

/** The CSS families of the fonts the console ships. The variables come from `app/fonts.ts`. */
export const FONT_FAMILIES: Record<string, string> = {
  inter: "var(--font-inter), ui-sans-serif, system-ui, sans-serif",
  "ibm-plex-sans": "var(--font-ibm-plex-sans), ui-sans-serif, system-ui, sans-serif",
  "source-serif": "var(--font-source-serif), ui-serif, Georgia, serif",
  "jetbrains-mono": "var(--font-jetbrains-mono), ui-monospace, SFMono-Regular, Menlo, monospace",
  system: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif",
}

export type DashboardLook = {
  /** Whether the tiles (and the charts) draw dark. */
  dark: boolean
  /** Classes for the dashboard container. */
  className: string
  /** Inline style for the container: the font. */
  style: React.CSSProperties | undefined
}

/**
 * The look of a dashboard. `appDark` is whether the console around it is dark.
 * A font the administrator no longer allows (`allowedFonts`) or the console does
 * not ship is not applied: the console's own font is used.
 */
export function dashboardLook(
  board: { theme?: string; font?: string } | undefined,
  appDark: boolean,
  allowedFonts?: readonly string[],
): DashboardLook {
  const theme = isTheme(board?.theme) ? board.theme : "auto"
  const dark = theme === "auto" ? appDark : theme === "dark"
  const className = theme === "dark" ? "dark" : theme === "light" ? "theme-light" : ""
  const font = board?.font
  const family = font && FONT_FAMILIES[font] && (!allowedFonts || allowedFonts.includes(font)) ? FONT_FAMILIES[font] : undefined
  return { dark, className, style: family ? { fontFamily: family } : undefined }
}
