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

/** `v` with at most `decimals` decimals (none trailing), thousands separated. */
export function formatNumber(v: number, decimals = 0): string {
  return v.toLocaleString("en-US", { maximumFractionDigits: decimals })
}

/** Compact number for axes: 1.2K, 3.5M, 2B; below 1,000 up to `decimals` decimals. */
export function formatCompactNumber(v: number, decimals = 0): string {
  const a = Math.abs(v)
  const fmt = (n: number) => n.toLocaleString("en-US", { maximumFractionDigits: 1 })
  if (a >= 1_000_000_000) return `${fmt(v / 1_000_000_000)}B`
  if (a >= 1_000_000) return `${fmt(v / 1_000_000)}M`
  if (a >= 1_000) return `${fmt(v / 1_000)}K`
  return formatNumber(v, decimals)
}
