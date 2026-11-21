const MONTH = /^\d{4}-\d{2}$/

/** Beyond this many monthly points, one label per month is unreadable. */
const MONTHLY_LABEL_LIMIT = 24

/**
 * Axis-label settings for a long monthly category axis ("2020-01" … "2024-11"):
 * one unrotated label per year, at its first month, reading just the year.
 * The month stays in the tooltip. `null` when the axis is not that shape.
 */
export function monthlyAxisLabel(categories: string[]): {
  interval: (index: number, value: string) => boolean
  formatter: (value: string) => string
  rotate: 0
} | null {
  if (categories.length <= MONTHLY_LABEL_LIMIT || !categories.every((c) => MONTH.test(c))) return null
  return {
    interval: (index, value) => index === 0 || value.endsWith("-01"),
    formatter: (value) => value.slice(0, 4),
    rotate: 0,
  }
}

/**
 * BI-8 review fix (SHOULD-FIX) R4: how many decimals the numbers of a chart
 * need, read from the data it draws. Whole numbers keep the look they always
 * had (0). Fractions get enough to tell axis ticks and tooltips apart: a share
 * between 0 and 1 (0.23 against 0.18) needs three, values from 1 to 100 two,
 * from 100 to 1,000 one; from 1,000 up the compact form (1.2K) already carries one.
 * Chosen from the values, not from the column's type, because a calculated
 * field's type is only known as "number" and a plain `avg` is fractional too.
 */
export function decimalsFor(values: readonly number[]): number {
  let max = 0
  let fractional = false
  for (const v of values) {
    if (!Number.isFinite(v)) continue
    if (!Number.isInteger(v)) fractional = true
    max = Math.max(max, Math.abs(v))
  }
  if (!fractional || max >= 1000) return 0
  if (max >= 100) return 1
  if (max >= 1) return 2
  return 3
}

/**
 * `v` with at most `decimals` decimals (none trailing), thousands separated.
 * `locale` (BI-17: the deployment's number locale) is `en-US` unless the
 * caller passes one, which is what every chart showed before.
 */
export function formatNumber(v: number, decimals = 0, locale = "en-US"): string {
  return v.toLocaleString(locale, { maximumFractionDigits: decimals })
}

/**
 * Compact number for axes: 1.2K, 3.5M, 2B; below 1,000 up to `decimals`
 * decimals. This is the `auto` style of `value-format.ts`.
 */
export function formatCompactNumber(v: number, decimals = 0, locale = "en-US"): string {
  const a = Math.abs(v)
  const fmt = (n: number) => n.toLocaleString(locale, { maximumFractionDigits: 1 })
  if (a >= 1_000_000_000) return `${fmt(v / 1_000_000_000)}B`
  if (a >= 1_000_000) return `${fmt(v / 1_000_000)}M`
  if (a >= 1_000) return `${fmt(v / 1_000)}K`
  return formatNumber(v, decimals, locale)
}

const ORDERED_PARTS = [
  String.raw`-?\d+(?:\.\d+)?`, // 7, 2026, 12.5
  String.raw`\d{4}-\d{2}(?:-\d{2})?(?:[ T].*)?`, // 2026-03, 2026-03-05, 2026-03-05 10:00
  String.raw`Q[1-4](?: \d{4})?`, // Q2, Q2 2026
  String.raw`W\d{1,2}`, // W14
  String.raw`\d{2}:\d{2}`, // 09:00
  "Mon|Tue|Wed|Thu|Fri|Sat|Sun", // day of week
  String.raw`(?:\d{1,2} )?(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)(?: \d{4}| \d{2}:\d{2})?`, // Mar, Mar 2026, 5 Mar, 5 Mar 10:00
]
const ORDERED_LABEL = new RegExp(`^(?:${ORDERED_PARTS.map((p) => `(?:${p})`).join("|")})$`)

/**
 * BI-73 (c): whether a category axis has an order a line can follow: dates,
 * months, years, quarters, weekdays, hours or plain numbers. A line over
 * unordered text categories (regions, products) is drawn with straight
 * segments, because a smooth curve would dip between points that have no
 * order. An empty axis is not ordered.
 */
export function isOrderedAxis(categories: readonly string[]): boolean {
  return categories.length > 0 && categories.every((c) => ORDERED_LABEL.test(c.trim()))
}
