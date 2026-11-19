/**
 * A report's blocks and Markdown, each way (`BI-62` PR B, decision D8).
 *
 * - `markdownToBlocks` turns what the assistant wrote into a draft into the
 *   report's blocks when the person accepts it. It reads the Markdown the
 *   editor can hold, and nothing else is lost or invented: anything it does
 *   not know becomes paragraph text.
 * - `blocksToMarkdown` is the report as the assistant reads it in the page
 *   context (charts as `[chart: <title> · <dashboard>]`).
 *
 * Pure functions, tested on every kind and on hostile input
 * (`report-markdown.test.ts`).
 *
 * # What `markdownToBlocks` reads
 *
 * - `# ` and `## ` headings (levels 1 and 2, what the editor writes); `###`
 *   to `######` become level 3, which the editor still reads and saves (D3),
 *   so a deeper heading is not turned into text with hashes in it.
 * - `- `, `* ` and `+ ` items as a bulleted list, `1. ` or `1) ` items as a
 *   numbered one. An indented item joins the same list (a report's list has
 *   one level, as `report-doc.ts` saves an indented item), and an indented
 *   line under an item continues it.
 * - `> ` lines as one quote (its lines joined by a newline).
 * - `---`, `***` or `___` as a divider.
 * - Every other line is paragraph text; lines next to each other are one
 *   paragraph (joined by a space, as Markdown reads a soft break), and a
 *   blank line starts the next one. A fenced code block is paragraph text as
 *   written (its marks escaped): the report has no code block.
 * - An answer wrapped whole in one ```` ```markdown ```` fence is read as the
 *   Markdown inside it.
 *
 * Inline marks go through `report-inline.ts`: `**bold**`, `*italic*` and
 * `[words](address)` stay marks, written back in the report's own subset; a
 * link whose address fails `safeReportUrl` becomes its plain words (the
 * parser drops the address). Nothing here produces HTML.
 *
 * The report's limits (300 blocks, 10,000 characters per text, 100 list
 * items) are the server's to apply: a draft over them is refused on save
 * with the server's sentence beside the status, and the text stays (D4).
 */
import type { ReportBlock } from "@/services/contracts/dashboard-reports"
import { escapeInline, parseInline, serializeInline } from "./report-inline"
import { safeReportUrl } from "./report-blocks"

/** One text in the report's inline subset, from Markdown inline text. */
function inlineText(markdown: string): string {
  return serializeInline(parseInline(markdown.trim()))
}

const HEADING = /^(#{1,6})[ \t]+(.*?)(?:[ \t]+#+)?[ \t]*$/
const BULLET = /^([ \t]*)[-*+][ \t]+(.*)$/
const NUMBERED = /^([ \t]*)\d{1,9}[.)][ \t]+(.*)$/
const QUOTE = /^[ \t]{0,3}>[ \t]?(.*)$/
const DIVIDER = /^[ \t]{0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/
const FENCE = /^[ \t]{0,3}(`{3,}|~{3,})/
const WHOLE_FENCE = /^(`{3,}|~{3,})[ \t]*(?:markdown|md)[ \t]*\n([\s\S]*?)\n\1[ \t]*$/i

type Open =
  | { kind: "paragraph"; lines: string[] }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "quote"; lines: string[] }
  | { kind: "code"; fence: string; lines: string[] }
  | null

/** What the assistant wrote, as the report's blocks. */
export function markdownToBlocks(markdown: string): ReportBlock[] {
  let source = markdown.replace(/\r\n?/g, "\n").trim()
  const wrapped = WHOLE_FENCE.exec(source)
  if (wrapped) source = wrapped[2]
  const blocks: ReportBlock[] = []
  let open: Open = null

  const close = () => {
    if (!open) return
    if (open.kind === "paragraph") {
      const text = inlineText(open.lines.join(" "))
      if (text) blocks.push({ type: "paragraph", text })
    } else if (open.kind === "list") {
      const items = open.items.map(inlineText).filter((i) => i !== "")
      if (items.length) blocks.push(open.ordered ? { type: "list", items, ordered: true } : { type: "list", items })
    } else if (open.kind === "quote") {
      const text = open.lines.map(inlineText).filter((l) => l !== "").join("\n")
      if (text) blocks.push({ type: "quote", text })
    } else {
      // Code is shown as written: every mark character escaped, line breaks kept.
      const text = open.lines.map(escapeInline).join("\n").trim()
      if (text) blocks.push({ type: "paragraph", text })
    }
    open = null
  }

  for (const line of source.split("\n")) {
    if (open?.kind === "code") {
      if (line.trim().startsWith(open.fence)) close()
      else open.lines.push(line)
      continue
    }
    const fence = FENCE.exec(line)
    if (fence) {
      close()
      open = { kind: "code", fence: fence[1], lines: [] }
      continue
    }
    if (line.trim() === "") {
      close()
      continue
    }
    // A divider before a list item: `* * *` and `- - -` are rules, not items.
    if (DIVIDER.test(line)) {
      close()
      blocks.push({ type: "divider" })
      continue
    }
    const heading = HEADING.exec(line)
    if (heading) {
      close()
      const level = heading[1].length === 1 ? 1 : heading[1].length === 2 ? 2 : 3
      const text = inlineText(heading[2])
      if (text) blocks.push({ type: "heading", text, level })
      continue
    }
    const quote = QUOTE.exec(line)
    if (quote) {
      if (open?.kind !== "quote") {
        close()
        open = { kind: "quote", lines: [] }
      }
      open.lines.push(quote[1])
      continue
    }
    const bullet = BULLET.exec(line)
    const numbered = bullet ? null : NUMBERED.exec(line)
    const item = bullet ?? numbered
    if (item) {
      const ordered = numbered !== null
      const nested = item[1].length > 0
      // An indented item stays in the list it is under, whatever its marker.
      if (open?.kind === "list" && (open.ordered === ordered || nested)) {
        open.items.push(item[2])
      } else {
        close()
        open = { kind: "list", ordered, items: [item[2]] }
      }
      continue
    }
    if (open?.kind === "list" && /^\s/.test(line)) {
      // An indented line under an item continues that item.
      open.items[open.items.length - 1] += ` ${line.trim()}`
      continue
    }
    if (open?.kind !== "paragraph") {
      close()
      open = { kind: "paragraph", lines: [] }
    }
    open.lines.push(line.trim())
  }
  close()
  return blocks
}

/** How a chart reads in the Markdown the assistant is sent. */
export type ChartLabel = { title: string; dashboard: string }

/** A chart as one line: `[chart: <title> · <dashboard>]`. */
export function chartLine(label: ChartLabel | undefined): string {
  if (!label) return "[chart: not available]"
  const title = label.title.replace(/\s+/g, " ").trim() || "Untitled chart"
  const dashboard = label.dashboard.replace(/\s+/g, " ").trim()
  return dashboard ? `[chart: ${title} · ${dashboard}]` : `[chart: ${title}]`
}

/**
 * The report's blocks as Markdown, for the assistant to read. Text is the
 * report's inline subset already, which is Markdown; a quote's lines each
 * start with `> `.
 */
export function blocksToMarkdown(blocks: readonly ReportBlock[], chartLabel: (id: string) => ChartLabel | undefined): string {
  const parts: string[] = []
  for (const block of blocks) {
    switch (block.type) {
      case "heading":
        parts.push(`${"#".repeat(block.level === 1 || block.level === 3 ? block.level : 2)} ${block.text}`)
        break
      case "paragraph":
        parts.push(block.text)
        break
      case "list":
        parts.push(block.items.map((item, i) => `${block.ordered ? `${i + 1}.` : "-"} ${item.replace(/\n/g, " ")}`).join("\n"))
        break
      case "quote":
        parts.push(
          block.text
            .split("\n")
            .map((l) => `> ${l}`)
            .join("\n"),
        )
        break
      case "divider":
        parts.push("---")
        break
      case "link": {
        const words = escapeInline(block.text || block.url)
        parts.push(safeReportUrl(block.url) ? `[${words}](${escapeInline(block.url.trim())})` : words)
        break
      }
      case "chart":
        parts.push(chartLine(chartLabel(block.chart)))
        break
    }
  }
  return parts.join("\n\n")
}
