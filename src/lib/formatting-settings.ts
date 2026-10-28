/**
 * The closed lists behind the deployment's formatting settings (`BI-17`
 * part A): the number locales, the fonts the console ships, and the bounds of
 * the company palette. Mirrors `lakehouse_bi::formatting` in the Rust crate;
 * the server validates against its own copy, this one only offers choices.
 */

export const NUMBER_LOCALES = {
  "en-US": "English (1,234.5)",
  "id-ID": "Indonesian (1.234,5)",
} as const

export const DEFAULT_NUMBER_LOCALE = "en-US"

/** The fonts the console ships, in the server's order; nothing loads from the internet. */
export const SHIPPED_FONTS = {
  inter: "Inter",
  "ibm-plex-sans": "IBM Plex Sans",
  "source-serif": "Source Serif",
  "jetbrains-mono": "JetBrains Mono",
  system: "System font",
} as const

export const SHIPPED_FONT_IDS = Object.keys(SHIPPED_FONTS)

export const COMPANY_PALETTE_MIN = 2
export const COMPANY_PALETTE_MAX = 12

/** The most content sites a deployment may list; mirrors `MAX_CONTENT_SITES`. */
export const MAX_CONTENT_SITES = 50

/** Most outside email domains a scheduled report may be allowed to (`RPT-1`, D8). */
export const MAX_REPORT_DOMAINS = 50

/** Whether `s` is `#rrggbb`, the only colour form the server accepts. */
export function isHexColor(s: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(s)
}
