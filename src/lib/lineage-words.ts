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
  const at = step.notRecorded || !step.refreshedAt ? null : new Date(step.refreshedAt)
  if (!at || Number.isNaN(at.getTime())) return "Not recorded"
  let when: string
  try {
    when = new Intl.DateTimeFormat("en-GB", { timeZone, dateStyle: "medium", timeStyle: "short" }).format(at)
  } catch {
    when = at.toISOString()
  }
  return step.refreshSource ? `Refreshed ${when} (${step.refreshSource})` : `Refreshed ${when}`
}

/** "and 3 more you do not have access to"; empty when none are hidden. */
export function hiddenWords(count: number): string {
  return count > 0 ? `and ${count.toLocaleString("en-US")} more you do not have access to` : ""
}
