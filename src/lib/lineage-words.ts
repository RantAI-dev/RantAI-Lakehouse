/**
 * The words of chart lineage (`BI-14`). The refusal sentence is the owner's
 * (questions-for-owner Q6, signed 2026-10-24); a user without `lineage:read`
 * sees it instead of the path, wherever the path would be.
 */
export const LINEAGE_REFUSED = "Lineage needs the lineage permission; ask your administrator."

/** The caption of a step's kind in the path. */
export const STEP_KIND_LABEL: Record<string, string> = {
  chart: "Chart",
  sql_source: "SQL source",
  metric: "Metric",
  gold: "Gold",
  silver: "Silver",
  bronze: "Bronze",
  connector: "Connector",
  upload: "Upload",
  pipeline: "Pipeline",
}

/**
 * When a step was last refreshed, in the report zone, or "Not recorded" when
 * there is no record (never a guess, feature decision 3).
 */
export function refreshWords(step: { refreshedAt?: string; refreshSource?: string; notRecorded?: boolean }, timeZone: string): string {
  if (!hasRefreshTime(step)) return "Not recorded"
  const at = new Date(step.refreshedAt ?? "")
  let when: string
  try {
    when = new Intl.DateTimeFormat("en-GB", { timeZone, dateStyle: "medium", timeStyle: "short" }).format(at)
  } catch {
    when = at.toISOString()
  }
  return step.refreshSource ? `Refreshed ${when} (${step.refreshSource})` : `Refreshed ${when}`
}

/** True when a step has a refresh time on record that can be read as a date. */
export function hasRefreshTime(step: { refreshedAt?: string; notRecorded?: boolean }): boolean {
  return !step.notRecorded && !!step.refreshedAt && !Number.isNaN(new Date(step.refreshedAt).getTime())
}

/**
 * The one quiet line under a chart's route (`BI-71` decision 1): what the
 * route leaves out, said once instead of "Not recorded" on every step and
 * "No column mapping recorded" under every field. The detail (the server's
 * `coverage` sentences) goes in a help hint after it. `null` when nothing is
 * missing and the server has nothing to add. The chart step is left out of
 * the count: a chart has no refresh of its own to record.
 */
export function missingWords(lineage: {
  steps: { kind: string; refreshedAt?: string; notRecorded?: boolean }[]
  fields: { columns: unknown[] }[]
  coverage: string[]
}): string | null {
  const sources = lineage.steps.filter((s) => s.kind !== "chart")
  const timed = sources.filter(hasRefreshTime).length
  const mapped = lineage.fields.filter((f) => f.columns.length > 0).length
  if (timed === 0 && mapped === 0) return "No refresh times or column mappings recorded yet"
  const missing = timed < sources.length || mapped < lineage.fields.length
  return missing || lineage.coverage.length > 0 ? "Only recorded times and columns are shown" : null
}

/** "and 3 more you do not have access to"; empty when none are hidden. */
export function hiddenWords(count: number): string {
  return count > 0 ? `and ${count.toLocaleString("en-US")} more you do not have access to` : ""
}
