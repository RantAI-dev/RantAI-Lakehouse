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
  mode: "none" | "previous" | "goal"
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
  const value = num(rows[last]?.v)
  const reference = rows.length > 1 ? num(rows[last - 1]?.v) : null
  const valueBucket = rows[last]?.[compare.dateColumn]
  const referenceBucket = rows.length > 1 ? rows[last - 1]?.[compare.dateColumn] : undefined
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

/** `+12,5%` / `-3%`, one decimal at most; `null` reads as nothing. */
export function signedPercent(percent: number | null): string {
  if (percent === null) return ""
  const body = Math.abs(percent).toLocaleString("id-ID", { maximumFractionDigits: 1 })
  return `${percent > 0 ? "+" : percent < 0 ? "-" : ""}${body}%`
}

/** `+1.234` / `-56`. */
export function signedAmount(delta: number): string {
  const body = Math.abs(delta).toLocaleString("id-ID", { maximumFractionDigits: 2 })
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
