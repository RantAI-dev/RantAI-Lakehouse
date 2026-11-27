import type { AlertRunResult } from "@/services/contracts/alerts"

/**
 * The one line the alerts page prints for a run's outcome. A rule that could
 * not be evaluated reads "Failed: <reason>" (`BI-67`; `failed` in
 * `lakehouse_alerts::RunResult`), a deliberate skip reads "skipped: <reason>".
 */
export function formatRunResult(r: AlertRunResult): string {
  if (r.failed) {
    return `Failed: ${r.failed}`
  }
  if (r.skipped) {
    return `skipped: ${r.skipped}`
  }
  if (r.type === "alert") {
    const formattedVal = Math.round(r.value ?? 0).toLocaleString()
    if (r.fired) {
      if (r.delivered?.ok) {
        return `fired (value ${formattedVal}) · sent`
      }
      const err = r.delivered?.error ?? "unknown error"
      return `fired (value ${formattedVal}) · send failed: ${err}`
    }
    return `ok (value ${formattedVal}, not breached)`
  }
  if (r.delivered?.ok) {
    return "digest sent"
  }
  const err = r.delivered?.error ?? "unknown error"
  return `send failed: ${err}`
}
