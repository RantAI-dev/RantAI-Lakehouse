import { formatCompactNumber } from "./chart-axis"
import type { ValueFormat } from "./chart-style"
import { formatCompactCurrency, formatValue } from "./value-format"

/**
 * How a big number fits its tile (owner QA 2026-11-15: "Total Foreign" showed
 * "3,330,172" because 13,330,172 at the KPI's size was wider than a 310 px
 * tile and was cut on both sides).
 *
 * The order of preference is fixed: the full number with its thousand
 * separators (DESIGN.md: big numbers always carry them) at the tile's own
 * size; else the same text a little smaller, never below `KPI_FIT_FLOOR` of
 * that size, so a row of KPIs still reads as headline numbers; and only when
 * even that is too wide, the compact form ("13.3 M"), whose full value the
 * tile then gives in a tooltip and to screen readers. Nothing is ever clipped
 * on purpose; a compact text too wide at the floor is the last resort, and it
 * ends in "…".
 */
export const KPI_FIT_FLOOR = 0.6

export type KpiFit = { readonly mode: "full" | "compact"; readonly scale: number }

export const KPI_FIT_NATURAL: KpiFit = { mode: "full", scale: 1 }

/**
 * The size and form of the number. All widths are CSS pixels: `available` is
 * the content width the number may use, `fullWidth` and `compactWidth` are
 * the widths of the two texts at the tile's base size (text width scales
 * linearly with font size, so one measure at the base size answers every
 * scale). `compactWidth` is null when the value has no shorter form.
 *
 * Nothing measured (a width of zero or less, as before layout or in a test
 * DOM) keeps the natural size: the tile must never shrink on a guess.
 */
export function fitKpi(available: number, fullWidth: number, compactWidth: number | null, floor = KPI_FIT_FLOOR): KpiFit {
  if (!(available > 0) || !(fullWidth > 0)) return KPI_FIT_NATURAL
  const full = available / fullWidth
  if (full >= 1) return KPI_FIT_NATURAL
  if (full >= floor) return { mode: "full", scale: full }
  if (compactWidth === null || !(compactWidth > 0) || compactWidth >= fullWidth) return { mode: "full", scale: floor }
  return { mode: "compact", scale: Math.min(1, Math.max(floor, available / compactWidth)) }
}

/** Two fits draw the same: a change under a hundredth of the size is not worth a render. */
export function sameKpiFit(a: KpiFit, b: KpiFit): boolean {
  return a.mode === b.mode && Math.abs(a.scale - b.scale) < 0.01
}

/**
 * The compact form of a KPI value under its own format, or null when it has
 * none shorter (below 1,000, or a style that is already short: percent,
 * scientific, compact). It goes through the console's one compact writer
 * (`value-format.ts`, decision 3: K, M, B, T in every locale, so "13,3 M" in
 * `id-ID`), with at most one decimal, and keeps the format's prefix, suffix
 * and currency.
 */
export function compactKpiText(v: number, fmt: ValueFormat | undefined, locale = "en-US", autoDecimals = 0): string | null {
  if (!Number.isFinite(v) || Math.abs(v) < 1000) return null
  const style = fmt?.style ?? "auto"
  switch (style) {
    case "auto":
      return `${fmt?.prefix ?? ""}${formatCompactNumber(v, autoDecimals, locale)}${fmt?.suffix ?? ""}`
    case "number":
      return formatValue(v, { ...fmt, style: "compact", decimals: undefined }, locale, autoDecimals)
    case "currency":
      return formatCompactCurrency(v, { ...fmt, decimals: undefined }, locale)
    default:
      return null
  }
}
