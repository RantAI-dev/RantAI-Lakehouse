import { formatCompactNumber, formatNumber } from "./chart-axis"
import type { DateFormat, StyleFields, ValueFormat } from "./chart-style"

/**
 * The one formatter every number and date on a chart goes through (`BI-17`
 * part A, D3): axis, tooltip, data label, KPI, table cell, reference line
 * label. The server never formats; it stores a `ValueFormat` and the console
 * reads it here, with the deployment's number locale (`en-US` 1,234.5 or
 * `id-ID` 1.234,5; Admin > Settings).
 *
 * Decisions this file carries out (feature page):
 * - `auto` is what the console showed before this existed (`chart-axis.ts`),
 *   so a chart with no format changes nowhere.
 * - Compact uses K, M, B, T in every locale (decision 3), not the locale's own
 *   words, so "Rp 1,2 M" in `id-ID`.
 * - Percent multiplies by 100 (0.125 is 12.5%); a column that already holds
 *   12.5 uses the `number` style with the suffix "%".
 */

/** Without this a currency symbol and its number are split by a no-break space the tests and the eye do not expect. */
function plainSpaces(s: string): string {
  return s.replace(/[  ]/g, " ")
}

function fraction(decimals: number | undefined, fallbackMax: number): Intl.NumberFormatOptions {
  return decimals === undefined
    ? { maximumFractionDigits: fallbackMax }
    : { minimumFractionDigits: decimals, maximumFractionDigits: decimals }
}

const COMPACT_STEPS: readonly [number, string][] = [
  [1e12, "T"],
  [1e9, "B"],
  [1e6, "M"],
  [1e3, "K"],
]

/**
 * The compact step of `v`: the scaled number and its letter, or the number
 * itself below 1,000.
 *
 * A value that rounds up to a thousand of its own step belongs to the next
 * one: 999,950 is "1 M", not "1,000 K"
 * (`PR #199 review M SHOULD-FIX BI-17 8`).
 */
function compactParts(v: number, decimals: number | undefined): { scaled: number; letter: string } {
  const a = Math.abs(v)
  const digits = decimals ?? 1
  for (const [index, [size, letter]] of COMPACT_STEPS.entries()) {
    if (a < size) continue
    const scaled = v / size
    // COMPACT_STEPS runs from the largest unit down, so the step above is the
    // one before it.
    if (index > 0 && Math.abs(scaled) >= 1000 - 0.5 * 10 ** -digits) {
      const [upSize, upLetter] = COMPACT_STEPS[index - 1]!
      return { scaled: v / upSize, letter: upLetter }
    }
    return { scaled, letter }
  }
  return { scaled: v, letter: "" }
}

/** The number part of a format, without prefix and suffix. */
function body(v: number, fmt: ValueFormat, locale: string, autoDecimals: number): string {
  const grouping = fmt.thousands !== false
  const style = fmt.style ?? "auto"
  switch (style) {
    case "number":
      return plainSpaces(v.toLocaleString(locale, { ...fraction(fmt.decimals, 3), useGrouping: grouping }))
    case "percent":
      return `${plainSpaces((v * 100).toLocaleString(locale, { ...fraction(fmt.decimals, 2), useGrouping: grouping }))}%`
    case "scientific":
      return plainSpaces(
        new Intl.NumberFormat(locale, { notation: "scientific", ...fraction(fmt.decimals, 2) }).format(v),
      )
    case "compact": {
      // Compact with a currency code is a compact amount: "Rp 1,23 M" (decision 3).
      if (fmt.currency) return compactCurrency(v, fmt, locale)
      const { scaled, letter } = compactParts(v, fmt.decimals)
      const n = plainSpaces(scaled.toLocaleString(locale, { ...fraction(fmt.decimals, 1), useGrouping: grouping }))
      return letter ? `${n} ${letter}` : n
    }
    case "currency": {
      const code = fmt.currency ?? "IDR"
      const display = fmt.currencyDisplay ?? "symbol"
      return plainSpaces(
        new Intl.NumberFormat(locale, {
          style: "currency",
          currency: code,
          currencyDisplay: display,
          useGrouping: grouping,
          ...(fmt.decimals === undefined ? {} : fraction(fmt.decimals, fmt.decimals)),
        }).format(v),
      )
    }
    default:
      return formatNumber(v, autoDecimals, locale)
  }
}

/** A currency in compact form: "Rp 1,23 M". The symbol and its place come from `Intl`; the scaled number carries the letter. */
function compactCurrency(v: number, fmt: ValueFormat, locale: string): string {
  const { scaled, letter } = compactParts(v, fmt.decimals)
  const decimals = fmt.decimals ?? (letter ? 1 : undefined)
  const text = plainSpaces(
    new Intl.NumberFormat(locale, {
      style: "currency",
      currency: fmt.currency ?? "IDR",
      currencyDisplay: fmt.currencyDisplay ?? "symbol",
      useGrouping: fmt.thousands !== false,
      ...(decimals === undefined ? {} : fraction(decimals, decimals)),
    }).format(scaled),
  )
  return letter ? `${text} ${letter}` : text
}

/**
 * `v` under `fmt`. `autoDecimals` is what the `auto` style uses (read from
 * the data by `decimalsFor`). A value that is not a finite number is
 * returned as text. A compact format with a currency code is a compact
 * amount; a currency format without `compact` is written in full.
 */
export function formatValue(
  v: number,
  fmt: ValueFormat | undefined,
  locale = "en-US",
  autoDecimals = 0,
): string {
  if (!Number.isFinite(v)) return String(v)
  const f = fmt ?? {}
  const text = body(v, f, locale, autoDecimals)
  return `${f.prefix ?? ""}${text}${f.suffix ?? ""}`
}

/**
 * A compact amount ("Rp 1,2 M") under the format's currency, decimals and
 * display: also what an axis uses for a currency where a long amount would
 * not fit.
 */
export function formatCompactCurrency(v: number, fmt: ValueFormat, locale = "en-US"): string {
  if (!Number.isFinite(v)) return String(v)
  return `${fmt.prefix ?? ""}${compactCurrency(v, fmt, locale)}${fmt.suffix ?? ""}`
}

/**
 * A value on an axis. `auto` keeps the compact 1.2K / 3.5M of before. A
 * number or currency format with no decimals set is shortened from 1,000 up so
 * the labels fit; an explicit decimals or style (`compact`, `percent`,
 * `scientific`) is honoured as set.
 */
export function formatAxisValue(
  v: number,
  fmt: ValueFormat | undefined,
  locale = "en-US",
  autoDecimals = 0,
): string {
  if (!Number.isFinite(v)) return String(v)
  const style = fmt?.style ?? "auto"
  if (style === "auto") {
    return `${fmt?.prefix ?? ""}${formatCompactNumber(v, autoDecimals, locale)}${fmt?.suffix ?? ""}`
  }
  if (style === "currency" && fmt && fmt.decimals === undefined && Math.abs(v) >= 1000) {
    return formatCompactCurrency(v, fmt, locale)
  }
  if (style === "number" && fmt && fmt.decimals === undefined && Math.abs(v) >= 1000) {
    return formatValue(v, { ...fmt, style: "compact" }, locale, autoDecimals)
  }
  return formatValue(v, fmt, locale, autoDecimals)
}

/** `2026-10-15` or `2026-10-15 14:05:09`: the engine prints the calendar day and the time in the report zone. */
const STAMP = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2}))?)?/

/**
 * A date or timestamp under `fmt`. Text that is not a date, and the `auto`
 * style, come back as they were. The calendar day and the clock are read off
 * the text and formatted in UTC, so the browser's own zone never moves them.
 */
export function formatDateValue(value: unknown, fmt: DateFormat | undefined, locale = "en-US"): string {
  const raw = String(value ?? "")
  const style = fmt?.style ?? "auto"
  const m = STAMP.exec(raw.trim())
  if (!m || (style === "auto" && fmt?.hour12 === undefined && !fmt?.abbreviate)) return raw
  const [, y, mo, d, hh, mi] = m
  const at = new Date(Date.UTC(Number(y), Number(mo) - 1, Number(d), Number(hh ?? 0), Number(mi ?? 0)))
  const month = fmt?.abbreviate ? "short" : "long"
  let day: string
  switch (style) {
    case "short":
      day = new Intl.DateTimeFormat(locale, { timeZone: "UTC", day: "2-digit", month: "2-digit", year: "2-digit" }).format(at)
      break
    case "long":
      day = new Intl.DateTimeFormat(locale, { timeZone: "UTC", day: "numeric", month, year: "numeric" }).format(at)
      break
    case "monthYear":
      day = new Intl.DateTimeFormat(locale, { timeZone: "UTC", month, year: "numeric" }).format(at)
      break
    case "iso":
      day = `${y}-${mo}-${d}`
      break
    case "auto":
      // Only the time or the abbreviation was set: keep the day as the engine wrote it.
      day = `${y}-${mo}-${d}`
      break
    default:
      day = new Intl.DateTimeFormat(locale, { timeZone: "UTC", day: "numeric", month: "short", year: "numeric" }).format(at)
  }
  if (hh === undefined) return day
  const time = new Intl.DateTimeFormat(locale, {
    timeZone: "UTC",
    // The locale's own clock when the format does not say: the server's own
    // doc says "absent follows the locale", and this forced 24-hour time
    // (`PR #199 review M SHOULD-FIX BI-17 8`).
    hour: fmt?.hour12 === false ? "2-digit" : "numeric",
    minute: "2-digit",
    hour12: fmt?.hour12,
  }).format(at)
  return `${day} ${plainSpaces(time)}`
}

/**
 * The format a metric's saved hint stands for (`BI-17`, D4). The hint is a
 * short word the metric's author typed (`MetricDef.format`), with no list
 * behind it, so only the words that are exactly a style here are honoured; any
 * other hint is left unread rather than guessed at ("unsupported, honestly").
 */
export function formatFromHint(hint: string | undefined): ValueFormat | undefined {
  switch ((hint ?? "").trim().toLowerCase()) {
    case "number": return { style: "number" }
    case "percent": return { style: "percent" }
    case "compact": return { style: "compact" }
    case "scientific": return { style: "scientific" }
    default: return undefined
  }
}

/**
 * A chart's style with its metrics' hints as the lowest default: a metric's
 * format applies to its value unless the chart has a format of its own or the
 * value has one (decision 1 and D4). Returns `style` itself when nothing is
 * added.
 */
export function withMetricFormats(style: StyleFields | undefined, hints: Record<string, string> | undefined): StyleFields | undefined {
  if (!hints || style?.format) return style
  const fromHints: Record<string, ValueFormat> = {}
  for (const [name, hint] of Object.entries(hints)) {
    const f = formatFromHint(hint)
    if (f) fromHints[name] = f
  }
  if (!Object.keys(fromHints).length) return style
  return { ...style, valueFormats: { ...fromHints, ...style?.valueFormats } }
}
