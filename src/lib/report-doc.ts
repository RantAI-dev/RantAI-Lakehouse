/**
 * The report editor's document and the report's blocks, each a view of the
 * other (`BI-62`, decision D3: storage stays a list of blocks; the editor is a
 * view of it). Pure functions, so the round trip is tested on every block kind
 * without an editor (`report-doc.test.ts`).
 *
 * The document is Tiptap JSON: a `title` line, then block nodes. Text goes
 * through the inline subset of `report-inline.ts` both ways, so a mark the
 * person makes is saved as `**bold**`, `*italic*` or `[words](address)` and a
 * character they type is escaped.
 *
 * What the editor cannot hold is mapped, never dropped silently:
 *
 * - An old `link` block (made before BI-62) becomes a paragraph holding an
 *   inline link, and is saved that way: the only lossless rewrite (D3).
 * - A level-3 heading still reads and writes as level 3; the editor only
 *   offers 1 and 2.
 * - A list item the person indented under another (Tab) is saved as the next
 *   item of the same list: a report's list has one level.
 *
 * And what holds nothing is not saved: an empty paragraph (a blank line), a
 * list or quote without words, a chart with no id, and an assistant's draft
 * (PR B), which is not part of the report until it is accepted.
 */
import type { ReportBlock } from "@/services/contracts/dashboard-reports"
import { escapeInline, inlineFromNodes, inlineToNodes, type InlineNode } from "./report-inline"
import { MAX_REPORT_CHARTS, safeReportUrl } from "./report-blocks"

/** A Tiptap JSON node, structurally (`JSONContent`). */
export type DocNode = {
  type: string
  attrs?: Record<string, unknown>
  content?: DocNode[]
  text?: string
  marks?: InlineNode["marks"]
}

/** The node name of a chart block in the editor. */
export const CHART_NODE = "chart"
/** The node name of the assistant's draft (PR B); never saved. */
export const DRAFT_NODE = "aiDraft"

function inline(text: string): DocNode[] {
  return inlineToNodes(text) as DocNode[]
}

function textBlock(type: string, text: string, attrs?: Record<string, unknown>): DocNode {
  const content = inline(text)
  return { type, ...(attrs ? { attrs } : {}), ...(content.length ? { content } : {}) }
}

/** The title and blocks as the editor's document. */
export function blocksToDoc(title: string, blocks: readonly ReportBlock[]): DocNode {
  const content: DocNode[] = [title ? { type: "title", content: [{ type: "text", text: title }] } : { type: "title" }]
  for (const block of blocks) {
    switch (block.type) {
      case "heading":
        content.push(textBlock("heading", block.text, { level: block.level === 1 || block.level === 3 ? block.level : 2 }))
        break
      case "paragraph":
        content.push(textBlock("paragraph", block.text))
        break
      case "list":
        content.push({
          type: block.ordered ? "orderedList" : "bulletList",
          content: (block.items.length ? block.items : [""]).map((item) => ({
            type: "listItem",
            content: [textBlock("paragraph", item)],
          })),
        })
        break
      case "quote":
        content.push({ type: "blockquote", content: [textBlock("paragraph", block.text)] })
        break
      case "divider":
        content.push({ type: "horizontalRule" })
        break
      case "link": {
        // D3: a link block is shown and saved as a paragraph holding the link.
        const words = escapeInline(block.text || block.url)
        content.push(textBlock("paragraph", safeReportUrl(block.url) ? `[${words}](${escapeInline(block.url.trim())})` : words))
        break
      }
      case "chart":
        content.push({ type: CHART_NODE, attrs: { chart: block.chart, width: block.width === "wide" ? "wide" : "text" } })
        break
    }
  }
  // The schema is `title block+`: a page always has a line to write on.
  if (content.length === 1) content.push({ type: "paragraph" })
  return { type: "doc", content }
}

/** The plain text of a node: its text nodes joined, a hard break as a newline. */
function plain(node: DocNode | undefined): string {
  if (!node) return ""
  if (node.type === "text") return node.text ?? ""
  if (node.type === "hardBreak") return "\n"
  return (node.content ?? []).map(plain).join("")
}

/** Every text block inside `node`, in order (a quote's paragraphs, a list item's lines). */
function textBlocksIn(node: DocNode): DocNode[] {
  if (node.type === "paragraph" || node.type === "heading") return [node]
  return (node.content ?? []).flatMap(textBlocksIn)
}

/** A list's items, a nested item following its parent in the same list. */
function listItems(list: DocNode): string[] {
  const out: string[] = []
  for (const item of list.content ?? []) {
    if (item.type !== "listItem") continue
    const lines: string[] = []
    for (const child of item.content ?? []) {
      if (child.type === "bulletList" || child.type === "orderedList") {
        if (lines.length) out.push(lines.join("\n"))
        lines.length = 0
        out.push(...listItems(child))
      } else {
        lines.push(...textBlocksIn(child).map((t) => inlineFromNodes(t.content)))
      }
    }
    if (lines.length) out.push(lines.join("\n"))
  }
  return out.filter((i) => i.trim() !== "")
}

/** The editor's document as the title and the blocks that are saved. */
export function docToBlocks(doc: DocNode): { title: string; blocks: ReportBlock[] } {
  const nodes = doc.content ?? []
  const first = nodes[0]
  const title = first?.type === "title" ? plain(first).replace(/\s+/g, " ").trim() : ""
  const blocks: ReportBlock[] = []
  for (const node of first?.type === "title" ? nodes.slice(1) : nodes) {
    switch (node.type) {
      case "heading": {
        const text = inlineFromNodes(node.content)
        const level = node.attrs?.level === 1 || node.attrs?.level === 3 ? (node.attrs.level as 1 | 3) : 2
        if (text.trim()) blocks.push({ type: "heading", text, level })
        break
      }
      case "paragraph": {
        const text = inlineFromNodes(node.content)
        if (text.trim()) blocks.push({ type: "paragraph", text })
        break
      }
      case "bulletList":
      case "orderedList": {
        const items = listItems(node)
        if (items.length) blocks.push(node.type === "orderedList" ? { type: "list", items, ordered: true } : { type: "list", items })
        break
      }
      case "blockquote": {
        const text = textBlocksIn(node)
          .map((t) => inlineFromNodes(t.content))
          .filter((t) => t.trim() !== "")
          .join("\n")
        if (text) blocks.push({ type: "quote", text })
        break
      }
      case "horizontalRule":
        blocks.push({ type: "divider" })
        break
      case CHART_NODE: {
        const chart = typeof node.attrs?.chart === "string" ? node.attrs.chart : ""
        if (chart) blocks.push(node.attrs?.width === "wide" ? { type: "chart", chart, width: "wide" } : { type: "chart", chart })
        break
      }
      default:
        // An assistant's draft (PR B) and anything else is not part of the report.
        break
    }
  }
  return { title, blocks }
}

/** The charts the document holds (each block counts, as the server counts them). */
export function docChartCount(doc: DocNode): number {
  return (doc.content ?? []).filter((n) => n.type === CHART_NODE).length
}

/** When the header starts showing "N of 30 charts". */
export function nearChartLimit(count: number): boolean {
  return count >= MAX_REPORT_CHARTS - 5
}

/* ------------------------------------------------------------------ */
/* The "/" menu                                                        */
/* ------------------------------------------------------------------ */

export type SlashGroup = "Write with AI" | "Text" | "Insert"

export type SlashItemId = "text" | "h1" | "h2" | "bullet" | "numbered" | "quote" | "chart" | "divider" | "ai"

export type SlashEntry = {
  id: SlashItemId
  label: string
  group: SlashGroup
  /** Words that also find it. */
  keywords: string
  /** The Markdown shortcut that makes the same block, shown quietly on the right. */
  hint?: string
}

/** The menu's groups, in screen order. */
export const SLASH_GROUPS: readonly SlashGroup[] = ["Write with AI", "Text", "Insert"]

/**
 * What "/" offers, in order. Every entry's hint is the shortcut the editor
 * really has (`report-editor-extensions.test.ts` checks them against a live
 * editor). "Ask AI to write…" (`AI_SLASH_ITEM`) comes first for a person who
 * may use the assistant.
 */
export const SLASH_ITEMS: readonly SlashEntry[] = [
  { id: "text", label: "Text", group: "Text", keywords: "paragraph plain" },
  { id: "h1", label: "Heading 1", group: "Text", keywords: "title big h1", hint: "#" },
  { id: "h2", label: "Heading 2", group: "Text", keywords: "subtitle h2", hint: "##" },
  { id: "bullet", label: "Bulleted list", group: "Text", keywords: "bullet unordered ul", hint: "-" },
  { id: "numbered", label: "Numbered list", group: "Text", keywords: "ordered ol 1.", hint: "1." },
  { id: "quote", label: "Quote", group: "Text", keywords: "blockquote cite", hint: ">" },
  { id: "chart", label: "Chart…", group: "Insert", keywords: "graph dashboard visual kpi table" },
  { id: "divider", label: "Divider", group: "Insert", keywords: "hr rule line separator", hint: "---" },
]

/** "/" → Ask AI to write… (PR B, D8): first in the menu, so it is seen without scrolling (mockup D). */
export const AI_SLASH_ITEM: SlashEntry = {
  id: "ai",
  label: "Ask AI to write…",
  group: "Write with AI",
  keywords: "ai copilot write draft summary summarise summarize assistant",
}

/** The items a query finds: by label or keyword, case-insensitive, in menu order. */
export function matchSlashItems<T extends { label: string; keywords: string }>(items: readonly T[], query: string): T[] {
  const q = query.trim().toLowerCase()
  if (!q) return [...items]
  return items.filter((i) => i.label.toLowerCase().includes(q) || i.keywords.toLowerCase().includes(q))
}
