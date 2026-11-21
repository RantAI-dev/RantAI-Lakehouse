/**
 * The inline marks of a report's text (`BI-62`, decision D3): `**bold**`,
 * `*italic*` and `[words](address)` inside the text of a heading, paragraph,
 * list item or quote, with `\` escaping `\ * [ ] ( )`.
 *
 * Why a Markdown subset in one string, not a new storage format: the schema,
 * the PDF view, the assistant's tools and every export keep one text field per
 * block; plain text reads the same; and the assistant already writes Markdown.
 * The editor escapes what a person types (`escapeInline`), so a literal `*`
 * they type stays a `*`.
 *
 * Nothing here produces HTML. `parseInline` turns a text into runs (words with
 * their marks), which the renderer (`report-blocks.tsx`) draws as React nodes
 * and the editor turns into Tiptap text nodes (`inlineToNodes`). A link is a
 * link only when its address passes `safeReportUrl` (the server's rule); any
 * other address is dropped and its words stay as text. The server checks every
 * address the same way (`lakehouse_bi::reports`, `BadInlineLink`) and applies
 * the link rule below at every unescaped `[`, so what it checks covers every
 * link this parser can make.
 *
 * # The rules, exactly (the server mirrors the link part)
 *
 * - A `\` before one of `\ * [ ] ( )` makes that character literal and is not
 *   shown; before anything else it is itself.
 * - A link starts at an unescaped `[`: the first unescaped `]` after it, an
 *   unescaped `(` right after that, and the first unescaped `)` after the `(`.
 *   Without all three, the `[` is text. A link's words never hold another link.
 * - A run of unescaped `*` that follows a character other than a space first
 *   closes what is open, innermost first (an italic takes one star, a bold
 *   two). Then, if a character other than a space follows it, it opens bold
 *   (two stars) and italic (one), each only if it is not already open. Stars
 *   left over are text, so `2 * 3` is text. An opener never closed is text, and
 *   its mark is taken back.
 *
 * `serializeInline` writes bold outside italic and a link outside both, which
 * is the order the star rule reads back, so `parseInline(serializeInline(r))`
 * is `r` up to `normalizeRuns` (whitespace at the edge of a bold or italic is
 * moved outside it, as Markdown readers expect).
 */
import { safeReportUrl } from "./report-blocks"

/** Words with their marks. `href` is set only for a safe address. */
export type InlineRun = {
  text: string
  bold?: boolean
  italic?: boolean
  href?: string
}

const ESCAPABLE = new Set(["\\", "*", "[", "]", "(", ")"])

/** Every character a person typed, escaped so it reads back as itself. */
export function escapeInline(text: string): string {
  let out = ""
  for (const ch of text) out += ESCAPABLE.has(ch) ? `\\${ch}` : ch
  return out
}

type Scan = {
  chars: string[]
  /** Character `i` is literal because a `\` escapes it. */
  escaped: boolean[]
  /** Character `i` is that `\` and is not shown. */
  escaper: boolean[]
  /**
   * The first unescaped `]` (and `)`) at or after `i`, or -1: precomputed so a
   * hostile text of ten thousand `[` is read in one pass, as on the server.
   */
  nextClose: number[]
  nextEnd: number[]
}

function scan(source: string): Scan {
  // Code points, not UTF-16 units, so positions agree with the server's `chars()`.
  const chars = Array.from(source)
  const escaped = new Array<boolean>(chars.length).fill(false)
  const escaper = new Array<boolean>(chars.length).fill(false)
  for (let i = 0; i < chars.length; ) {
    if (chars[i] === "\\" && i + 1 < chars.length && ESCAPABLE.has(chars[i + 1])) {
      escaper[i] = true
      escaped[i + 1] = true
      i += 2
    } else {
      i += 1
    }
  }
  const next = (want: string) => {
    const out = new Array<number>(chars.length + 1).fill(-1)
    for (let at = chars.length - 1; at >= 0; at -= 1) {
      out[at] = chars[at] === want && !escaped[at] ? at : out[at + 1]
    }
    return out
  }
  return { chars, escaped, escaper, nextClose: next("]"), nextEnd: next(")") }
}

type Mark = "bold" | "italic"

type Item =
  | { kind: "text"; text: string; boldBy: number | null; italicBy: number | null }
  | { kind: "delim"; id: number; stars: string; boldBy: number | null; italicBy: number | null }
  | { kind: "link"; runs: InlineRun[]; href: string; boldBy: number | null; italicBy: number | null }

function unescapedAt(s: Scan, at: number, want: string): boolean {
  return s.chars[at] === want && !s.escaped[at]
}

/** The first position at or after `from` in `next`, if it is before `to`; else -1. */
function firstBefore(next: readonly number[], from: number, to: number): number {
  const at = from < next.length ? next[from] : -1
  return at >= 0 && at < to ? at : -1
}

function shown(s: Scan, from: number, to: number): string {
  let out = ""
  for (let at = from; at < to; at += 1) if (!s.escaper[at]) out += s.chars[at]
  return out
}

/** The runs of `s` between `from` and `to`; links only when `links`. */
function parseRange(s: Scan, from: number, to: number, links: boolean): InlineRun[] {
  const items: Item[] = []
  const stack: { id: number; mark: Mark }[] = []
  let bold: number | null = null
  let italic: number | null = null
  let nextId = 0
  const text = (t: string) => items.push({ kind: "text", text: t, boldBy: bold, italicBy: italic })

  for (let i = from; i < to; ) {
    if (s.escaper[i]) {
      i += 1
      continue
    }
    const ch = s.chars[i]
    if (ch === "*" && !s.escaped[i]) {
      let j = i
      while (j < to && unescapedAt(s, j, "*")) j += 1
      let left = j - i
      // Only a run after a word closes and only a run before a word opens,
      // so `*a **b** c*` nests and `2 * 3` stays text.
      const afterWord = i > from && !/\s/.test(s.chars[i - 1])
      const beforeWord = j < to && !/\s/.test(s.chars[j])
      while (afterWord) {
        const top = stack[stack.length - 1]
        if (top?.mark === "italic" && left >= 1) {
          stack.pop()
          italic = null
          left -= 1
        } else if (top?.mark === "bold" && left >= 2) {
          stack.pop()
          bold = null
          left -= 2
        } else break
      }
      if (beforeWord && left >= 2 && bold === null) {
        const id = nextId++
        items.push({ kind: "delim", id, stars: "**", boldBy: bold, italicBy: italic })
        stack.push({ id, mark: "bold" })
        bold = id
        left -= 2
      }
      if (beforeWord && left >= 1 && italic === null) {
        const id = nextId++
        items.push({ kind: "delim", id, stars: "*", boldBy: bold, italicBy: italic })
        stack.push({ id, mark: "italic" })
        italic = id
        left -= 1
      }
      if (left > 0) text("*".repeat(left))
      i = j
      continue
    }
    if (ch === "[" && !s.escaped[i] && links) {
      const close = firstBefore(s.nextClose, i + 1, to)
      if (close >= 0 && close + 1 < to && unescapedAt(s, close + 1, "(")) {
        const end = firstBefore(s.nextEnd, close + 2, to)
        if (end >= 0) {
          items.push({
            kind: "link",
            runs: parseRange(s, i + 1, close, false),
            href: shown(s, close + 2, end),
            boldBy: bold,
            italicBy: italic,
          })
          i = end + 1
          continue
        }
      }
    }
    text(ch)
    i += 1
  }

  // Openers still on the stack were never closed: they are text, and the
  // words after them lose their mark.
  const open = new Set(stack.map((o) => o.id))
  const on = (by: number | null) => by !== null && !open.has(by)
  const runs: InlineRun[] = []
  for (const item of items) {
    if (item.kind === "delim") {
      if (open.has(item.id)) runs.push(run(item.stars, on(item.boldBy), on(item.italicBy)))
    } else if (item.kind === "text") {
      runs.push(run(item.text, on(item.boldBy), on(item.italicBy)))
    } else {
      const safe = safeReportUrl(item.href)
      const words = item.runs.some((r) => r.text !== "") ? item.runs : [{ text: item.href }]
      for (const r of words) {
        const out = run(r.text, Boolean(r.bold) || on(item.boldBy), Boolean(r.italic) || on(item.italicBy))
        if (safe) out.href = item.href
        runs.push(out)
      }
    }
  }
  return mergeRuns(runs)
}

function run(text: string, bold: boolean, italic: boolean): InlineRun {
  const out: InlineRun = { text }
  if (bold) out.bold = true
  if (italic) out.italic = true
  return out
}

function sameMarks(a: InlineRun, b: InlineRun): boolean {
  return Boolean(a.bold) === Boolean(b.bold) && Boolean(a.italic) === Boolean(b.italic) && a.href === b.href
}

/** Adjacent runs with the same marks joined, empty runs dropped. */
function mergeRuns(runs: readonly InlineRun[]): InlineRun[] {
  const out: InlineRun[] = []
  for (const r of runs) {
    if (r.text === "") continue
    const last = out[out.length - 1]
    if (last && sameMarks(last, r)) last.text += r.text
    else out.push({ ...r })
  }
  return out
}

/** A text as runs: words with their bold, italic and (safe) link marks. */
export function parseInline(source: string): InlineRun[] {
  if (source === "") return []
  const s = scan(source)
  return parseRange(s, 0, s.chars.length, true)
}

/** The words of a text without its marks (search, word counts, page context). */
export function inlinePlainText(source: string): string {
  return parseInline(source)
    .map((r) => r.text)
    .join("")
}

/**
 * The runs as they read back: merged, an unsafe link's address dropped, and
 * whitespace at the edge of a bold or italic moved outside it.
 */
export function normalizeRuns(runs: readonly InlineRun[]): InlineRun[] {
  const out: InlineRun[] = []
  for (const r of runs) {
    const href = r.href !== undefined && safeReportUrl(r.href) ? r.href : undefined
    const base: InlineRun = href === undefined ? { text: "" } : { text: "", href }
    if (!r.bold && !r.italic) {
      out.push({ ...base, text: r.text })
      continue
    }
    const m = /^(\s*)([\s\S]*?)(\s*)$/.exec(r.text)
    const [lead, core, trail] = m ? [m[1], m[2], m[3]] : ["", r.text, ""]
    out.push({ ...base, text: lead })
    out.push({ ...run(core, Boolean(r.bold), Boolean(r.italic)), ...(href === undefined ? {} : { href }) })
    out.push({ ...base, text: trail })
  }
  return mergeRuns(out)
}

const STARS: Record<Mark, string> = { bold: "**", italic: "*" }

/** One stretch of runs with the same link (or none), bold outside italic. */
function serializeMarks(runs: readonly InlineRun[]): string {
  let out = ""
  let stack: Mark[] = []
  for (const r of runs) {
    const want: Mark[] = []
    if (r.bold) want.push("bold")
    if (r.italic) want.push("italic")
    let keep = 0
    while (keep < stack.length && keep < want.length && stack[keep] === want[keep]) keep += 1
    for (let k = stack.length - 1; k >= keep; k -= 1) out += STARS[stack[k]]
    for (let k = keep; k < want.length; k += 1) out += STARS[want[k]]
    stack = want
    out += escapeInline(r.text)
  }
  for (let k = stack.length - 1; k >= 0; k -= 1) out += STARS[stack[k]]
  return out
}

/** Runs as one text in the subset; an unsafe link is written as its words. */
export function serializeInline(runs: readonly InlineRun[]): string {
  const norm = normalizeRuns(runs)
  let out = ""
  for (let i = 0; i < norm.length; ) {
    const href = norm[i].href
    let j = i
    while (j < norm.length && norm[j].href === href) j += 1
    const inner = serializeMarks(norm.slice(i, j))
    out += href === undefined ? inner : `[${inner}](${escapeInline(href)})`
    i = j
  }
  return out
}

/**
 * A Tiptap inline node, structurally (`JSONContent`): a text with marks, or a
 * hard break. Typed here so this module does not depend on the editor.
 */
export type InlineNode = {
  type: string
  text?: string
  marks?: { type: string; attrs?: Record<string, unknown> }[]
}

/** A Tiptap paragraph's (or heading's) content as one text in the subset. */
export function inlineFromNodes(nodes: readonly InlineNode[] | undefined): string {
  const runs: InlineRun[] = []
  for (const node of nodes ?? []) {
    if (node.type === "hardBreak") {
      runs.push({ text: "\n" })
      continue
    }
    if (node.type !== "text" || !node.text) continue
    const r: InlineRun = { text: node.text }
    for (const mark of node.marks ?? []) {
      if (mark.type === "bold") r.bold = true
      else if (mark.type === "italic") r.italic = true
      else if (mark.type === "link" && typeof mark.attrs?.href === "string") r.href = mark.attrs.href
    }
    runs.push(r)
  }
  return serializeInline(runs)
}

/** A text in the subset as Tiptap inline nodes; a newline is a hard break. */
export function inlineToNodes(source: string): InlineNode[] {
  const out: InlineNode[] = []
  for (const r of parseInline(source)) {
    const marks: NonNullable<InlineNode["marks"]> = []
    if (r.bold) marks.push({ type: "bold" })
    if (r.italic) marks.push({ type: "italic" })
    if (r.href !== undefined) marks.push({ type: "link", attrs: { href: r.href } })
    r.text.split("\n").forEach((part, i) => {
      if (i > 0) out.push({ type: "hardBreak" })
      if (part) out.push(marks.length ? { type: "text", text: part, marks } : { type: "text", text: part })
    })
  }
  return out
}
