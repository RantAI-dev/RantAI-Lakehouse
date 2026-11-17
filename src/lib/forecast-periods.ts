/**
 * The calendar arithmetic a forecast needs on the buckets of a grouped chart
 * (BI-17 part B, plan `2026-10-24-bi-17b-forecast-clustering.md`, D4).
 *
 * The server cuts the buckets in the report time zone and sends each as wall
 * clock text (`YYYY-MM-DD`, or `YYYY-MM-DD HH:MM:SS` for hour and minute;
 * `time-grain.ts`). So the next buckets are found by plain calendar steps on
 * that text, with no zone database, and always in the zone the data is in.
 * "Naive milliseconds" below means a wall-clock reading treated as UTC: two
 * of them compare correctly with each other and mean nothing as instants.
 * The one place a zone is needed is "what time is it now there"
 * (`wallClockMs`), because whether the last bucket is finished depends on it.
 */
import type { Grain } from "./time-grain"
import { bucketLabel } from "./time-grain"

export type Bucket = { y: number; mo: number; d: number; h: number; mi: number }

/** The grains whose buckets are a run of equal calendar steps; a part of the date (weekday, month of year) is not. */
export const FORECAST_GRAINS: readonly Grain[] = ["minute", "hour", "day", "week", "month", "quarter", "year"]

const RE = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2}))?/
/** More periods than this and the grid is refused rather than built (a minute grain over years). */
const MAX_PERIODS = 20000

export function parseBucket(value: unknown): Bucket | null {
  const m = RE.exec(String(value ?? ""))
  if (!m) return null
  const b = { y: Number(m[1]), mo: Number(m[2]), d: Number(m[3]), h: Number(m[4] ?? 0), mi: Number(m[5] ?? 0) }
  const t = new Date(Date.UTC(b.y, b.mo - 1, b.d, b.h, b.mi))
  // Reject 2026-02-31 and the like instead of letting Date roll it over.
  return t.getUTCMonth() === b.mo - 1 && t.getUTCDate() === b.d ? b : null
}

const naive = (b: Bucket) => Date.UTC(b.y, b.mo - 1, b.d, b.h, b.mi)
const fromNaive = (ms: number): Bucket => {
  const t = new Date(ms)
  return { y: t.getUTCFullYear(), mo: t.getUTCMonth() + 1, d: t.getUTCDate(), h: t.getUTCHours(), mi: t.getUTCMinutes() }
}

/** The bucket `n` periods after `b` (before, for a negative `n`). */
export function addPeriods(grain: Grain, b: Bucket, n: number): Bucket {
  switch (grain) {
    case "minute": return fromNaive(naive(b) + n * 60_000)
    case "hour": return fromNaive(naive(b) + n * 3_600_000)
    case "day": return fromNaive(naive(b) + n * 86_400_000)
    case "week": return fromNaive(naive(b) + n * 7 * 86_400_000)
    case "month": return fromNaive(Date.UTC(b.y, b.mo - 1 + n, 1, b.h, b.mi))
    case "quarter": return fromNaive(Date.UTC(b.y, b.mo - 1 + 3 * n, 1, b.h, b.mi))
    case "year": return fromNaive(Date.UTC(b.y + n, 0, 1, b.h, b.mi))
    default: return b
  }
}

const pad = (n: number) => String(n).padStart(2, "0")

/** A bucket as the server writes it, so `bucketLabel` reads it the same way. */
export function bucketText(grain: Grain, b: Bucket): string {
  const date = `${String(b.y).padStart(4, "0")}-${pad(b.mo)}-${pad(b.d)}`
  return grain === "hour" || grain === "minute" ? `${date} ${pad(b.h)}:${pad(b.mi)}:00` : date
}

/** Where a bucket ends (exclusive), in naive milliseconds: the start of the next one. */
export function bucketEnd(grain: Grain, b: Bucket): number {
  return naive(addPeriods(grain, b, 1))
}

/** The wall clock in `timeZone` at the instant `now` (epoch ms), as naive milliseconds. */
export function wallClockMs(now: number, timeZone: string): number {
  let parts: Intl.DateTimeFormatPart[]
  try {
    parts = new Intl.DateTimeFormat("en-CA", {
      timeZone, hourCycle: "h23", year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit",
    }).formatToParts(new Date(now))
  } catch {
    // An unknown zone name: read the clock as UTC rather than guess an offset.
    return now
  }
  const get = (type: string) => Number(parts.find((p) => p.type === type)?.value ?? 0)
  return Date.UTC(get("year"), get("month") - 1, get("day"), get("hour"), get("minute"))
}

export type Periods = {
  /** Every bucket from the first to the last, as the server writes them. */
  starts: string[]
  /** The value of each, `null` where the rows have no bucket. */
  values: (number | null)[]
  last: Bucket
}

/**
 * Lay the rows of a chart on a regular grid of periods. Returns `null` when
 * the buckets cannot be read as one run of dates (a text or part-of-date
 * dimension, rows out of order, the same bucket twice, an absurdly long span),
 * so the caller can say so instead of forecasting something else.
 */
export function layPeriods(grain: Grain, buckets: readonly unknown[], values: readonly number[]): Periods | null {
  if (!FORECAST_GRAINS.includes(grain) || buckets.length === 0 || buckets.length !== values.length) return null
  const parsed = buckets.map(parseBucket)
  if (parsed.some((b) => b === null)) return null
  const bs = parsed as Bucket[]
  // Walk the grid from the first bucket. A row must land exactly on a slot: one
  // that falls behind it is out of order, repeated or off the grain's boundary.
  const starts: string[] = []
  const grid: (number | null)[] = []
  let at = bs[0]
  let i = 0
  while (i < bs.length) {
    if (starts.length >= MAX_PERIODS) return null
    const text = bucketText(grain, at)
    starts.push(text)
    if (bucketText(grain, bs[i]) === text) {
      grid.push(values[i])
      i++
    } else if (naive(bs[i]) < naive(at)) {
      return null
    } else {
      grid.push(null)
    }
    at = addPeriods(grain, at, 1)
  }
  return { starts, values: grid, last: bs[bs.length - 1] }
}

/** The next `count` buckets after `last`, as axis labels (`Mar 2026`, `Q1 2027`, ...). */
export function futureLabels(grain: Grain, last: Bucket, count: number): string[] {
  return Array.from({ length: count }, (_, k) => bucketLabel(grain, bucketText(grain, addPeriods(grain, last, k + 1))))
}
