/**
 * A Workday report as the connector stores it (`SRC-3` slice 6): the text `owner/report_name` in the dial's
 * `objects` and as a source object's `name`. Mirrors `check_report` in
 * `dagster/dispar_orchestrate/saas/vendors/workday.py` and the connection test's `workday_report_path`
 * (`lakehouse-api/src/connector_probe/saas.rs`), which are the authority: the owner and the name go into a
 * request path, so only plain characters are accepted. The tenant and the host are the connector's account,
 * never part of this text.
 */

const OWNER = /^[A-Za-z0-9][A-Za-z0-9_.@-]{0,99}$/
const REPORT = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,149}$/

/** The stored text of a report. */
export function reportName(owner: string, report: string): string {
  return `${owner.trim()}/${report.trim()}`
}

/** A stored report text as its owner and name, or `null` for text that is not one. */
export function parseReportName(text: string): { owner: string; report: string } | null {
  const slash = text.indexOf("/")
  if (slash < 0) return null
  const owner = text.slice(0, slash)
  const report = text.slice(slash + 1)
  return reportProblem(owner, report, []) === null ? { owner, report } : null
}

/** Why the report cannot be added, in words, or `null`. `taken` are the stored texts already in use. */
export function reportProblem(owner: string, report: string, taken: readonly string[]): string | null {
  const o = owner.trim()
  const r = report.trim()
  if (!OWNER.test(o) || o.includes("..")) return "The owner is the report owner's Workday user name: letters, digits and . _ @ -"
  if (!REPORT.test(r) || r.includes("..")) return "Write the report name as it is in its address, with underscores for spaces: letters, digits and . _ -"
  if (taken.includes(reportName(o, r))) return "That report is already on the list."
  return null
}
