/**
 * A Google Analytics 4 report as the connector stores it (`SRC-3`, slice 3 review BLOCKER 9):
 * one text, `label = dimension, dimension | metric, metric`, in the dial's `objects` and as a
 * source object's `name`. Mirrors `parse_report` in
 * `dagster/dispar_orchestrate/saas/vendors/ga4.py`, which is the authority: the server refuses
 * anything this file would let through.
 *
 * The time dimension is the cursor and is never left out: when the person picks none, `date`
 * is added (the same rule as the server's).
 */

/** The dimensions that carry the report's time, in the order the server checks them (`TIME_DIMENSIONS`). */
export const GA4_TIME_DIMENSIONS = ["date", "isoYearIsoWeek", "year", "yearMonth", "dateHour", "dateHourMinute"] as const

const LABEL = /^[A-Za-z][A-Za-z0-9_]{0,63}$/
const API_NAME = /^[A-Za-z][A-Za-z0-9_]*(:[A-Za-z0-9_]+)?$/
/**
 * The Data API's own limits, checked separately (PR #216 final review SHOULD-FIX 22): Google's
 * discovery document for v1beta states both on `RunReportRequest` ("Requests are allowed up to 9
 * dimensions." / "Requests are allowed up to 10 metrics."); the "Create a report" guide says "you
 * can use up to nine dimensions per request". The time dimension counts as one.
 */
export const GA4_MAX_DIMENSIONS = 9
export const GA4_MAX_METRICS = 10

export type Ga4Report = { label: string; dimensions: string[]; metrics: string[] }

/** The dimensions of a report with its time dimension: the person's own, or `date` added last. */
export function withTimeDimension(dimensions: readonly string[]): string[] {
  return dimensions.some((d) => (GA4_TIME_DIMENSIONS as readonly string[]).includes(d)) ? [...dimensions] : [...dimensions, "date"]
}

/** The first time dimension of a report: its cursor. */
export function cursorOf(dimensions: readonly string[]): string {
  return withTimeDimension(dimensions).find((d) => (GA4_TIME_DIMENSIONS as readonly string[]).includes(d)) ?? "date"
}

/** The report's text, as stored. */
export function reportText({ label, dimensions, metrics }: Ga4Report): string {
  return `${label.trim()} = ${withTimeDimension(dimensions).join(", ")} | ${metrics.join(", ")}`
}

/** A stored report text as its parts, or `null` for text that is not one. */
export function parseReportText(text: string): Ga4Report | null {
  const eq = text.indexOf("=")
  const bar = text.indexOf("|", eq + 1)
  if (eq < 0 || bar < 0) return null
  const names = (part: string) => part.split(",").map((n) => n.trim()).filter(Boolean)
  const report = { label: text.slice(0, eq).trim(), dimensions: names(text.slice(eq + 1, bar)), metrics: names(text.slice(bar + 1)) }
  return reportProblem(report, []) === null ? report : null
}

/** Why the report cannot be saved, in words, or `null`. `taken` are the labels already in use. */
export function reportProblem(report: Ga4Report, taken: readonly string[]): string | null {
  const label = report.label.trim()
  if (!LABEL.test(label)) return "Name the report with letters, digits and underscores, starting with a letter."
  if (taken.includes(label)) return "A report with this name exists."
  if (report.metrics.length === 0) return "Choose at least one metric."
  const dimensions = withTimeDimension(report.dimensions)
  if (dimensions.length > GA4_MAX_DIMENSIONS)
    return `A report takes at most ${GA4_MAX_DIMENSIONS} dimensions, including its date.`
  if (report.metrics.length > GA4_MAX_METRICS) return `A report takes at most ${GA4_MAX_METRICS} metrics.`
  const all = [...dimensions, ...report.metrics]
  if (!all.every((n) => API_NAME.test(n))) return "A dimension or metric has a name Google Analytics does not use."
  return null
}
