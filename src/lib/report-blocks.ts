/**
 * The pure rules of a report's blocks on the console side (`BI-18` part D,
 * R1 and R4): the link rule the server applies, the chart limit, and the edits
 * the block editor makes. The server checks all of it again
 * (`lakehouse_bi::reports`); these exist so the editor can say no before a save.
 */
import type { ReportBlock, ReportBlockType } from "@/services/contracts/dashboard-reports"

/** The most chart blocks a report may hold (decision R4); the server's sentence names it. */
export const MAX_REPORT_CHARTS = 30

/** The server's sentence for the 31st chart. */
export const TOO_MANY_CHARTS = "A report holds at most 30 charts."

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

/** Whether one more chart may be added. */
export function canAddChart(blocks: readonly ReportBlock[]): boolean {
  return chartCount(blocks) < MAX_REPORT_CHARTS
}

/** A new, empty block of a kind (a chart block is made from a picked chart, not here). */
export function emptyBlock(type: Exclude<ReportBlockType, "chart">): ReportBlock {
  switch (type) {
    case "heading":
      return { type: "heading", text: "", level: 2 }
    case "paragraph":
      return { type: "paragraph", text: "" }
    case "list":
      return { type: "list", items: [""], ordered: false }
    case "link":
      return { type: "link", text: "", url: "https://" }
  }
}

/** `blocks` with `block` inserted after index `after` (`-1` puts it first). */
export function insertBlock(blocks: readonly ReportBlock[], after: number, block: ReportBlock): ReportBlock[] {
  const at = Math.max(0, Math.min(blocks.length, after + 1))
  return [...blocks.slice(0, at), block, ...blocks.slice(at)]
}

/** `blocks` with the block at `index` moved one place up (`-1`) or down (`1`); unchanged at an end. */
export function moveBlock(blocks: readonly ReportBlock[], index: number, by: -1 | 1): ReportBlock[] {
  const to = index + by
  if (index < 0 || index >= blocks.length || to < 0 || to >= blocks.length) return [...blocks]
  const next = [...blocks]
  const [moved] = next.splice(index, 1)
  next.splice(to, 0, moved)
  return next
}

export function removeBlock(blocks: readonly ReportBlock[], index: number): ReportBlock[] {
  return blocks.filter((_, i) => i !== index)
}

export function replaceBlock(blocks: readonly ReportBlock[], index: number, block: ReportBlock): ReportBlock[] {
  return blocks.map((b, i) => (i === index ? block : b))
}

/** A list's items as the editor's text box holds them: one item per line. */
export function listText(items: readonly string[]): string {
  return items.join("\n")
}

/** The text box's lines as list items; a box left empty is one empty item. */
export function listItems(text: string): string[] {
  const items = text.split("\n")
  return items.length ? items : [""]
}

/**
 * The blocks as they are saved: a list's blank lines dropped, a chart block
 * without the dashboard the server fills in anyway. Nothing else is changed;
 * the text is the person's own.
 */
export function blocksForSave(blocks: readonly ReportBlock[]): ReportBlock[] {
  return blocks.map((b) => {
    if (b.type === "list") return { ...b, items: b.items.filter((i) => i.trim() !== "") }
    if (b.type === "chart") return { type: "chart", chart: b.chart }
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
