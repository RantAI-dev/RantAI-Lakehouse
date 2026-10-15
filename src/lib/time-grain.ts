/**
 * Time grain (`BI-9`) in the console: which grains a column offers, how a
 * bucket the server returned is labelled on an axis, and which days a bucket
 * covers. The server cuts the buckets (`lakehouse-bi` `grain.rs`, in the
 * deployment's report time zone); this module only reads what it returned, so
 * it never needs the zone. A truncation arrives as `YYYY-MM-DD` (day and
 * coarser) or `YYYY-MM-DD HH:MM:SS` (minute, hour) and a part as a number.
 */

export type Grain =
  | "minute" | "hour" | "day" | "week" | "month" | "quarter" | "year"
  | "hour_of_day" | "day_of_week" | "day_of_month" | "week_of_year" | "month_of_year" | "quarter_of_year"

export type WeekStart = "monday" | "sunday"

/** What a column holds; mirrors `ColumnKind` in `lakehouse-bi` (lowercase on the wire). */
export type ColumnKind = "number" | "date" | "datetime" | "text"

/** Every grain, in the order the picker offers them. */
export const GRAINS: readonly Grain[] = [
  "minute", "hour", "day", "week", "month", "quarter", "year",
  "hour_of_day", "day_of_week", "day_of_month", "week_of_year", "month_of_year", "quarter_of_year",
]

export const TRUNCATIONS: readonly Grain[] = GRAINS.slice(0, 7)

export const GRAIN_LABELS: Record<Grain, string> = {
  minute: "Minute", hour: "Hour", day: "Day", week: "Week", month: "Month", quarter: "Quarter", year: "Year",
  hour_of_day: "Hour of day", day_of_week: "Day of week", day_of_month: "Day of month",
  week_of_year: "Week of year", month_of_year: "Month of year", quarter_of_year: "Quarter of year",
}

export function isGrain(value: unknown): value is Grain {
  return typeof value === "string" && (GRAINS as readonly string[]).includes(value)
}

export function isTruncation(grain: Grain): boolean {
  return (TRUNCATIONS as readonly Grain[]).includes(grain)
}

/** Needs a timestamp: a plain date has no hour or minute. */
export function needsTime(grain: Grain): boolean {
  return grain === "minute" || grain === "hour" || grain === "hour_of_day"
}

/**
 * The kind of a `ClickHouse` type string, as the server decides it
 * (`ColumnKind::from_clickhouse_type`): `Nullable` and `LowCardinality`
 * unwrap, `Int*`/`UInt*`/`Float*`/`Decimal*` are numbers, `Date` and
 * `Date32` dates, `DateTime*` timestamps, everything else text.
 */
export function columnKindOfType(type: string): ColumnKind {
  let t = type.trim()
  for (;;) {
    const inner = /^(?:Nullable|LowCardinality)\((.*)\)$/.exec(t)
    if (!inner) break
    t = inner[1].trim()
  }
  if (/^(?:U?Int\d+|Float\d+|BFloat16)$/.test(t) || /^Decimal(?:\d+)?(?:\(.*\))?$/.test(t)) return "number"
  if (t === "Date" || t === "Date32") return "date"
  if (t === "DateTime" || t.startsWith("DateTime(") || t.startsWith("DateTime64")) return "datetime"
  return "text"
}

/** Chart kinds that take a grain; a calendar takes `day` only. Mirrors `Grain::kind_takes`. A pivot's grain applies to its first date row or column field (BI-16 part A). */
const GRAIN_KINDS: ReadonlySet<string> = new Set([
  "bar", "hbar", "line", "area", "stacked", "combo", "waterfall", "heatmap", "pie", "rose", "funnel",
  "treemap", "radar", "calendar", "pivot",
])

export function kindTakesGrain(chartKind: string): boolean {
  return GRAIN_KINDS.has(chartKind)
}

/** Whether `grain` applies to a chart of `chartKind` whose dimension is `column`. */
export function grainFits(grain: Grain, chartKind: string, column: ColumnKind | undefined): boolean {
  if (!kindTakesGrain(chartKind)) return false
  if (chartKind === "calendar" && grain !== "day") return false
  if (column === "datetime") return true
  return column === "date" && !needsTime(grain)
}

/** What the "Group by" select offers for a column and chart kind; empty when it should not show. */
export function grainChoices(chartKind: string, column: ColumnKind | undefined): Grain[] {
  return GRAINS.filter((g) => grainFits(g, chartKind, column))
}

/** What a dashboard-wide switch offers: day to year, plus hour and minute only when every grained chart is on a timestamp (feature page decision 9). */
export function switchChoices(columns: readonly (ColumnKind | undefined)[]): Grain[] {
  const allTimestamps = columns.length > 0 && columns.every((c) => c === "datetime")
  return TRUNCATIONS.filter((g) => !needsTime(g) || allTimestamps)
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]

/** The label of an empty (NULL) bucket. */
export const NO_DATE_LABEL = "No date"

const DATE_RE = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2}))?/

/**
 * A bucket as an axis label: `Mar 2026`, `Q1 2026`, `2026`, `Mon`, `Jan`,
 * a week by its first day (`9 Mar 2026`), an hour `9 Mar 14:00`. A day keeps
 * its ISO text (the calendar and the records list read it as is). Anything
 * that does not have the grain's shape is returned as text rather than
 * invented.
 */
export function bucketLabel(grain: Grain, value: unknown): string {
  if (value === null || value === undefined || value === "") return NO_DATE_LABEL
  const text = String(value)
  const part = Number(text)
  switch (grain) {
    case "hour_of_day": return Number.isInteger(part) ? `${String(part).padStart(2, "0")}:00` : text
    case "day_of_week": return Number.isInteger(part) && part >= 1 && part <= 7 ? WEEKDAYS[part - 1] : text
    case "day_of_month": return Number.isInteger(part) ? String(part) : text
    case "week_of_year": return Number.isInteger(part) ? `W${part}` : text
    case "month_of_year": return Number.isInteger(part) && part >= 1 && part <= 12 ? MONTHS[part - 1] : text
    case "quarter_of_year": return Number.isInteger(part) ? `Q${part}` : text
    default: break
  }
  const m = DATE_RE.exec(text)
  if (!m) return text
  const [, y, mo, d, hh, mm] = m
  const month = MONTHS[Number(mo) - 1]
  if (!month) return text
  switch (grain) {
    case "year": return y
    case "quarter": return `Q${Math.floor((Number(mo) - 1) / 3) + 1} ${y}`
    case "month": return `${month} ${y}`
    case "week": return `${Number(d)} ${month} ${y}`
    case "hour": return hh ? `${Number(d)} ${month} ${hh}:00` : text
    case "minute": return hh ? `${Number(d)} ${month} ${hh}:${mm}` : text
    default: return text
  }
}

const pad = (n: number) => String(n).padStart(2, "0")
const iso = (t: Date) => `${t.getUTCFullYear()}-${pad(t.getUTCMonth() + 1)}-${pad(t.getUTCDate())}`

/**
 * The first and last day a bucket covers, for day and coarser: a month is
 * its first to its last day (February of a leap year ends on the 29th), a
 * quarter three months, a week seven days from its first day, which the
 * server already placed on the configured first day of the week, so the
 * setting needs no second reading here. `null` for minute, hour, a part, or
 * text that is not a date.
 */
export function bucketRange(grain: Grain, value: unknown): { from: string; to: string } | null {
  const m = DATE_RE.exec(String(value ?? ""))
  if (!m || !["day", "week", "month", "quarter", "year"].includes(grain)) return null
  const y = Number(m[1])
  const mo = Number(m[2]) - 1
  const d = Number(m[3])
  const start = new Date(Date.UTC(y, mo, d))
  if (Number.isNaN(start.getTime()) || start.getUTCMonth() !== mo) return null
  const end = new Date(start)
  switch (grain) {
    case "week": end.setUTCDate(d + 6); break
    case "month": end.setUTCMonth(mo + 1, 0); break
    case "quarter": end.setUTCMonth(mo + 3, 0); break
    case "year": end.setUTCMonth(12, 0); break
    default: break
  }
  return { from: iso(start), to: iso(end) }
}

/** Whether a click on a bucket can set a dashboard date range (feature page decision 3). */
export function bucketFiltersDashboard(grain: Grain): boolean {
  return ["day", "week", "month", "quarter", "year"].includes(grain)
}

/** Whether a click on a bucket can list its rows: a truncation can, a part of the date cannot. */
export function bucketListsRecords(grain: Grain): boolean {
  return isTruncation(grain)
}

/** The sentence the tile menu gives for a part of the date, which offers neither. */
export const PART_NO_CLICK_REASON = "A part of the date, such as a weekday, spans many days, so it has no rows to list or range to filter."

/** The Settings the console reads with the dashboard (`reporting`). */
export type ReportingContext = { timeZone: string; weekStart: WeekStart | string }

/** Calendar first day for ECharts (`firstDay`: 0 is Sunday, 1 is Monday). */
export function calendarFirstDay(weekStart: string | undefined): 0 | 1 {
  return weekStart === "sunday" ? 0 : 1
}

/** The most buckets a grouped chart may ask for; mirrors `MAX_GRAIN_LIMIT` in `grain.rs`. */
export const MAX_GRAIN_LIMIT = 1000

/** The builder's default limit, the one a person has not touched. */
export const DEFAULT_LIMIT = 20

/**
 * BI-9 review fix (SHOULD-FIX) R2: the limit after a grain is chosen. A
 * truncation left at the untouched default would show 20 days and a cut-off
 * mark, so it starts at the grained maximum; a limit the person set is kept.
 */
export function limitAfterGrainPick(next: Grain | "", current: number, touched: boolean): number {
  return next && isTruncation(next) && !touched && current === DEFAULT_LIMIT ? MAX_GRAIN_LIMIT : current
}

/**
 * Whether the limit field does anything: a part of the date ordered by bucket
 * returns its whole range whatever the limit (R1), so the field is not shown.
 */
export function limitHasEffect(grain: Grain | "", order: string): boolean {
  return !(grain && !isTruncation(grain) && order === "none")
}
