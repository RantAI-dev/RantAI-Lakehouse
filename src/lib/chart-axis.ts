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

/** Compact number for axes: 1.2K, 3.5M, 2B. */
export function formatCompactNumber(v: number): string {
  const a = Math.abs(v)
  const fmt = (n: number) => n.toLocaleString("en-US", { maximumFractionDigits: 1 })
  if (a >= 1_000_000_000) return `${fmt(v / 1_000_000_000)}B`
  if (a >= 1_000_000) return `${fmt(v / 1_000_000)}M`
  if (a >= 1_000) return `${fmt(v / 1_000)}K`
  return Math.round(v).toLocaleString("en-US")
}
