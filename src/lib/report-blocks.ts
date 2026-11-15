/**
 * The pure rules of a report's blocks on the console side (`BI-18` part D,
 * R1 and R4): the link rule the server applies, the chart limit, and the shape
 * a save sends. The server checks all of it again (`lakehouse_bi::reports`);
 * these exist so the editor can say no before a save. The document editor of
 * `BI-62` edits a Tiptap document instead of these blocks
 * (`report-doc.ts`), so the old per-block edits are gone.
 */
import type { ReportBlock } from "@/services/contracts/dashboard-reports"

/** The most chart blocks a report may hold (decision R4); the server's sentence names it. */
export const MAX_REPORT_CHARTS = 30

/**
 * Whether a link's address may be opened: the web, an email, or a page of this
 * console (`/...`, not `//host`). The same rule as `lakehouse_bi::reports`, so a
 * `javascript:` address is never a link even if a stored row somehow held one.
 */
export function safeReportUrl(url: string): boolean {
  const u = url.trim()
  const lower = u.toLowerCase()
  return (
    lower.startsWith("https://") ||
    lower.startsWith("http://") ||
    lower.startsWith("mailto:") ||
    (u.startsWith("/") && !u.startsWith("//"))
  )
}

export function chartCount(blocks: readonly ReportBlock[]): number {
  return blocks.filter((b) => b.type === "chart").length
}

/**
 * The blocks as they are saved: a list's blank lines dropped, a chart block
 * without the dashboard the server fills in anyway, and its width only when it
 * is wide (`BI-62`: the text width is the default the server does not write).
 * Nothing else is changed; the text is the person's own.
 */
export function blocksForSave(blocks: readonly ReportBlock[]): ReportBlock[] {
  return blocks.map((b) => {
    if (b.type === "list") return { ...b, items: b.items.filter((i) => i.trim() !== "") }
    if (b.type === "chart") return b.width === "wide" ? { type: "chart", chart: b.chart, width: "wide" } : { type: "chart", chart: b.chart }
    return b
  })
}

/**
 * What the report page tells the assistant (`BI-18` D review round 1,
 * SHOULD-FIX 2), the way the dashboard page names its board: the report's id
 * and title, so "add this chart to the report" can name the report. The title
 * is the person's own text, quoted and cut at 200 characters; nothing else of
 * the report is sent.
 */
export function buildReportPageContext(report: { id: string; title: string; chartCount: number }): string {
  const title = JSON.stringify(report.title.slice(0, 200))
  return [
    `The user is reading the report ${title} (report id ${report.id}, ${report.chartCount} of ${MAX_REPORT_CHARTS} charts).`,
    `To change this report, call update_report with id ${report.id}: \`chart\` adds one chart at the end, \`append\` adds blocks, \`blocks\` replaces them.`,
    "list_reports lists the other reports the user can read.",
  ].join(" ")
}
