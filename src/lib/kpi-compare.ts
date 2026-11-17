import { formatNumber } from "./chart-axis"
import type { KpiCompare } from "./table-types"

/**
 * What a KPI with a comparison shows (`BI-16` part A). With a previous period
 * the server returns the latest periods that have data, oldest first
 * (`kpi_trend_sql`): the last is the value, the one before it the comparison,
 * all of them the trend line. With a goal it returns the plain number.
 */
export type KpiReading = {
  /** The big number; `null` when the server returned no row. */
  value: number | null
  /** What it is compared with: the previous period's value, or the goal. */
  reference: number | null
  mode: "none" | "previous" | "last_year" | "goal"
  /** `value - reference`: the change, or the distance to the goal (negative below it). */
  delta: number | null
  /** Change in percent of the previous value, or percent of the goal reached. `null` when the reference is zero (decision: the amount only). */
  percent: number | null
  /** Better, worse or the same, by `goodDirection`. */
  tone: "good" | "bad" | "flat"
  /** The periods' values, oldest first, for the trend line. */
  series: number[]
  /** The bucket of the value and of the comparison, as the server printed them. */
  valueBucket?: unknown
  referenceBucket?: unknown
  /** `last_year` only: the change against the period before the latest one, so both comparisons show (BI-2-AC3). */
  previous?: { reference: number; delta: number; percent: number | null; tone: KpiReading["tone"]; bucket?: unknown } | null
}

function num(v: unknown): number | null {
  const n = typeof v === "number" ? v : typeof v === "string" && v.trim() !== "" ? Number(v) : Number.NaN
  return Number.isFinite(n) ? n : null
}

function toneOf(delta: number, good: "up" | "down"): KpiReading["tone"] {
  if (delta === 0) return "flat"
  return (delta > 0) === (good === "up") ? "good" : "bad"
}

/**
 * Read a KPI tile's rows under its definition. `dateColumn` names the bucket
 * column of a previous-period result.
 */
export function readKpi(
  rows: readonly Record<string, unknown>[],
  compare: KpiCompare | undefined,
  good: "up" | "down" = "up",
  /** The period the server cut the buckets by (`grain` of the tile): a week is shifted by 52 weeks, the rest by a year. */
  period?: string,
): KpiReading {
  if (!compare) {
    return { value: num(rows[0]?.v), reference: null, mode: "none", delta: null, percent: null, tone: "flat", series: [] }
  }
  if (compare.kind === "goal") {
    const value = num(rows[0]?.v)
    const goal = compare.value
    if (value === null) return { value, reference: goal, mode: "goal", delta: null, percent: null, tone: "flat", series: [] }
    const delta = value - goal
    // Reaching the goal is "good" in the good direction: at or above it for up, at or below it for down.
    const tone: KpiReading["tone"] = delta === 0 ? "good" : toneOf(delta, good)
    return { value, reference: goal, mode: "goal", delta, percent: goal === 0 ? null : (value / goal) * 100, tone, series: [] }
  }
  const series = rows.map((r) => num(r.v)).filter((v): v is number => v !== null)
  const last = rows.length - 1
  // A metric's comparison may leave the date column out: the bucket is the column that is not `v`.
  const bucketKey = compare.dateColumn ?? Object.keys(rows[0] ?? {}).find((k) => k !== "v") ?? ""
  if (compare.kind === "last_year") return readLastYear(rows, bucketKey, good, series, period ?? compare.period)
  const value = num(rows[last]?.v)
  const reference = rows.length > 1 ? num(rows[last - 1]?.v) : null
  const valueBucket = rows[last]?.[bucketKey]
  const referenceBucket = rows.length > 1 ? rows[last - 1]?.[bucketKey] : undefined
  if (value === null || reference === null) {
    return { value, reference, mode: "previous", delta: null, percent: null, tone: "flat", series, valueBucket, referenceBucket }
  }
  const delta = value - reference
  return {
    value, reference, mode: "previous", delta,
    // A previous value of zero has no percent change; the amount stands alone.
    percent: reference === 0 ? null : (delta / Math.abs(reference)) * 100,
    tone: toneOf(delta, good), series, valueBucket, referenceBucket,
  }
}

/** The bucket `n` years or weeks before `bucket` (`YYYY-MM-DD`), as the server shifts it: a calendar year, or 52 weeks for a week. */
export function yearAgoKey(bucket: unknown, period: string | undefined): string | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(String(bucket ?? ""))
  if (!m) return null
  if (period === "week") {
    const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])))
    d.setUTCDate(d.getUTCDate() - 52 * 7)
    return d.toISOString().slice(0, 10)
  }
  return `${Number(m[1]) - 1}-${m[2]}-${m[3]}`
}

/**
 * The same-period-last-year reading (BI-2): the latest period against the one a year before it,
 * found by its bucket (the server returns every period back to then, gaps leave the year-ago value
 * empty), plus the change against the period before the latest one.
 */
function readLastYear(rows: readonly Record<string, unknown>[], bucketKey: string, good: "up" | "down", series: number[], period: string | undefined): KpiReading {
  const last = rows.length - 1
  const value = num(rows[last]?.v)
  const valueBucket = rows[last]?.[bucketKey]
  const key = yearAgoKey(valueBucket, period)
  const match = key ? rows.find((r, i) => i < last && String(r[bucketKey] ?? "").slice(0, 10) === key) : undefined
  const reference = num(match?.v)
  const referenceBucket = match?.[bucketKey]
  const before = rows.length > 1 ? num(rows[last - 1]?.v) : null
  const previous =
    value !== null && before !== null
      ? { reference: before, delta: value - before, percent: before === 0 ? null : ((value - before) / Math.abs(before)) * 100, tone: toneOf(value - before, good), bucket: rows[last - 1]?.[bucketKey] }
      : null
  if (value === null || reference === null) {
    return { value, reference, mode: "last_year", delta: null, percent: null, tone: "flat", series, valueBucket, referenceBucket, previous }
  }
  const delta = value - reference
  return {
    value, reference, mode: "last_year", delta,
    percent: reference === 0 ? null : (delta / Math.abs(reference)) * 100,
    tone: toneOf(delta, good), series, valueBucket, referenceBucket, previous,
  }
}

/** `+12.5%` / `-3%`, one decimal at most; `null` reads as nothing. */
export function signedPercent(percent: number | null): string {
  if (percent === null) return ""
  const body = Math.abs(percent).toLocaleString("en-US", { maximumFractionDigits: 1 })
  return `${percent > 0 ? "+" : percent < 0 ? "-" : ""}${body}%`
}

/** `+1,234` / `-56`; fractional data keeps `decimals` as the chart does (`decimalsFor`). */
export function signedAmount(delta: number, decimals = 0): string {
  const body = formatNumber(Math.abs(delta), decimals)
  return `${delta > 0 ? "+" : delta < 0 ? "-" : ""}${body}`
}

/** Points for an SVG polyline, scaled into `width` x `height` with a margin; one value is a flat line. */
export function sparklinePoints(series: readonly number[], width: number, height: number, margin = 2): string {
  if (series.length === 0) return ""
  const min = Math.min(...series)
  const max = Math.max(...series)
  const span = max - min
  const x = (i: number) => (series.length === 1 ? width / 2 : margin + (i * (width - 2 * margin)) / (series.length - 1))
  const y = (v: number) => (span === 0 ? height / 2 : height - margin - ((v - min) * (height - 2 * margin)) / span)
  return series.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(" ")
}
