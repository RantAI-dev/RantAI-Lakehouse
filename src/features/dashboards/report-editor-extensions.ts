/**
 * The Tiptap pieces of the report editor (`BI-62`, mockup D, plan T4).
 *
 * - The document is `title block+`: the big title is the page's first line,
 *   not a form field, so Enter in it goes on to the text.
 * - Headings are this module's own node: levels 1 and 2 are written (`#`,
 *   `##`), a stored level 3 still reads, and each renders one level below the
 *   page's title (`h2`, `h3`, `h4`) with `data-level`, as the reader does, so
 *   one stylesheet styles both (`report-document.css`).
 * - `MoveBlock`: Alt+↑ / Alt+↓ moves the block holding the cursor (or the
 *   selected chart) one place, the keyboard twin of the ⋮⋮ handle.
 * - The "/" menu runs on `@tiptap/suggestion`; the menu itself is React
 *   (`report-editor-menus.tsx`), fed through a small store.
 * - A chart dragged from the Charts tab lands between blocks (`createShelfDrop`).
 * - The assistant's draft (`BI-62` PR B, D8) is a block of its own that is
 *   never part of the report: `insertDraft`, `removeDraft` and `acceptDraft`
 *   put it in and take it out without leaving a trace in the document.
 *
 * Every package is MIT, pinned at 3.34.0. No Tiptap Pro and no drag-handle
 * extension (it pulls collaboration packages): the handles are ours.
 */
import { Extension, Node, mergeAttributes, textblockTypeInputRule, type AnyExtension, type Editor, type JSONContent, type Range } from "@tiptap/core"
import { closeHistory } from "@tiptap/pm/history"
import type { Node as PmNode } from "@tiptap/pm/model"
import { NodeSelection, Plugin, PluginKey, TextSelection } from "@tiptap/pm/state"
import type { EditorView } from "@tiptap/pm/view"
import Suggestion, { type SuggestionKeyDownProps, type SuggestionProps } from "@tiptap/suggestion"
import { StarterKit } from "@tiptap/starter-kit"
import { Typography } from "@tiptap/extension-typography"
import { Focus, Placeholder } from "@tiptap/extensions"

import { safeReportUrl } from "@/lib/report-blocks"
import { DRAFT_NODE, blocksToDoc, matchSlashItems, type SlashEntry } from "@/lib/report-doc"
import type { ReportBlock } from "@/services/contracts/dashboard-reports"

/* ------------------------------------------------------------------ */
/* Document, title and headings                                        */
/* ------------------------------------------------------------------ */

export const ReportDocument = Node.create({
  name: "doc",
  topNode: true,
  content: "title block+",
})

export const Title = Node.create({
  name: "title",
  content: "text*",
  marks: "",
  defining: true,
  parseHTML: () => [{ tag: "h1[data-report-title]" }],
  renderHTML: () => ["h1", { "data-report-title": "", class: "rd-title" }, 0],
  addKeyboardShortcuts() {
    return {
      // Enter in the title goes to the first line of text (made when the next block is not text).
      Enter: ({ editor }) => {
        const { $from } = editor.state.selection
        if ($from.parent.type.name !== "title") return false
        const after = $from.after(1)
        const next = editor.state.doc.nodeAt(after)
        if (next && next.isTextblock) return editor.chain().setTextSelection(after + 1).scrollIntoView().run()
        return editor.chain().insertContentAt(after, { type: "paragraph" }).setTextSelection(after + 1).run()
      },
    }
  },
})

const HEADING_TAG: Record<number, string> = { 1: "h2", 2: "h3", 3: "h4" }

export const ReportHeading = Node.create({
  name: "heading",
  group: "block",
  content: "inline*",
  defining: true,
  addAttributes() {
    return { level: { default: 2, rendered: false } }
  },
  parseHTML() {
    return [
      { tag: "[data-level]", getAttrs: (el) => ({ level: Math.min(3, Math.max(1, Number((el as HTMLElement).dataset.level) || 2)) }) },
      { tag: "h1", attrs: { level: 1 } },
      { tag: "h2", attrs: { level: 1 } },
      { tag: "h3", attrs: { level: 2 } },
      { tag: "h4", attrs: { level: 3 } },
      { tag: "h5", attrs: { level: 3 } },
      { tag: "h6", attrs: { level: 3 } },
    ]
  },
  renderHTML({ node, HTMLAttributes }) {
    const level = node.attrs.level === 1 || node.attrs.level === 3 ? (node.attrs.level as number) : 2
    return [HEADING_TAG[level], mergeAttributes(HTMLAttributes, { "data-level": String(level) }), 0]
  },
  addKeyboardShortcuts() {
    return {
      "Mod-Alt-1": () => this.editor.commands.toggleNode(this.name, "paragraph", { level: 1 }),
      "Mod-Alt-2": () => this.editor.commands.toggleNode(this.name, "paragraph", { level: 2 }),
    }
  },
  addInputRules() {
    // `# ` and `## ` only: the editor writes levels 1 and 2 (plan D3).
    return [1, 2].map((level) =>
      textblockTypeInputRule({ find: new RegExp(`^(#{${level}})\\s$`), type: this.type, getAttributes: { level } }),
    )
  },
})

/* ------------------------------------------------------------------ */
/* Moving blocks                                                       */
/* ------------------------------------------------------------------ */

/** Move the top-level block holding the selection one place; the title never moves and nothing passes above it. */
export function moveBlock(editor: Editor, dir: -1 | 1): boolean {
  const { state, view } = editor
  const { selection, doc } = state
  const index = selection.$from.index(0)
  if (index === 0) return false
  const target = index + dir
  if (target < 1 || target >= doc.childCount) return true
  let start = 0
  for (let i = 0; i < index; i++) start += doc.child(i).nodeSize
  const node = doc.child(index)
  const other = doc.child(target)
  const tr = state.tr
  let newStart: number
  if (dir === -1) {
    const otherStart = start - other.nodeSize
    tr.replaceWith(otherStart, start + node.nodeSize, [node, other])
    newStart = otherStart
  } else {
    tr.replaceWith(start, start + node.nodeSize + other.nodeSize, [other, node])
    newStart = start + other.nodeSize
  }
  const delta = newStart - start
  if (selection instanceof NodeSelection) {
    tr.setSelection(NodeSelection.create(tr.doc, selection.from + delta))
  } else {
    const end = start + node.nodeSize
    const to = selection.to <= end ? selection.to + delta : selection.from + delta
    tr.setSelection(TextSelection.create(tr.doc, selection.from + delta, to))
  }
  view.dispatch(tr.scrollIntoView())
  return true
}

export const MoveBlock = Extension.create({
  name: "moveBlock",
  addKeyboardShortcuts() {
    return {
      "Alt-ArrowUp": ({ editor }) => moveBlock(editor, -1),
      "Alt-ArrowDown": ({ editor }) => moveBlock(editor, 1),
    }
  },
})

/**
 * A click back into the text after a button inside the document had focus (a
 * chart's toolbar): ProseMirror, regaining focus, puts back the selection it
 * last had a moment later, which would undo the click. So the click's own
 * position is made the selection first.
 */
export const ClickBack = Extension.create({
  name: "clickBack",
  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey("clickBack"),
        props: {
          handleDOMEvents: {
            mousedown(view, event) {
              const active = document.activeElement
              const target = event.target as HTMLElement | null
              if (!active || active === view.dom || !view.dom.contains(active)) return false
              if (target?.closest("[contenteditable=false]")) return false
              const at = view.posAtCoords({ left: event.clientX, top: event.clientY })
              if (!at) return false
              view.dispatch(view.state.tr.setSelection(TextSelection.near(view.state.doc.resolve(at.pos))))
              view.focus()
              return false
            },
          },
        },
      }),
    ]
  },
})

/* ------------------------------------------------------------------ */
/* Inserting a block                                                   */
/* ------------------------------------------------------------------ */

/**
 * Put a block node where the person asked for it: at `at` (a gap between
 * blocks); on an empty line, in that line's place; otherwise under the block
 * holding the cursor. Never above the title.
 */
export function insertBlock(editor: Editor, node: JSONContent, at?: number) {
  const { state } = editor
  const titleEnd = state.doc.child(0).nodeSize
  if (typeof at === "number") {
    editor.chain().insertContentAt(Math.max(at, titleEnd), node).run()
    return
  }
  const { $from } = state.selection
  if (state.selection instanceof NodeSelection && $from.depth === 0) {
    editor.chain().insertContentAt(Math.max(state.selection.to, titleEnd), node).run()
    return
  }
  if ($from.depth === 0) {
    editor.chain().insertContentAt(Math.max($from.pos, titleEnd), node).run()
    return
  }
  const top = $from.node(1)
  const from = $from.before(1)
  const to = $from.after(1)
  if (top.type.name === "paragraph" && top.content.size === 0) editor.chain().insertContentAt({ from, to }, node).run()
  else editor.chain().insertContentAt(Math.max(to, titleEnd), node).run()
}

/* ------------------------------------------------------------------ */
/* The "/" menu                                                        */
/* ------------------------------------------------------------------ */

export type SlashItem = SlashEntry & {
  disabled?: boolean
  /** Why a disabled item cannot be picked now, in one line (its tooltip). */
  reason?: string
  run: (editor: Editor, range: Range) => void
}

export type SlashState = {
  open: boolean
  items: SlashItem[]
  query: string
  rect: DOMRect | null
  command: ((item: SlashItem) => void) | null
}

/** A small external store: Tiptap's suggestion callbacks write it, the React menu reads it. */
export class SlashStore {
  state: SlashState = { open: false, items: [], query: "", rect: null, command: null }
  private listeners = new Set<() => void>()
  /** Set by the mounted menu: it owns the highlighted row, so it answers ↑ ↓ Enter. */
  keyHandler: ((e: KeyboardEvent) => boolean) | null = null
  subscribe = (l: () => void) => {
    this.listeners.add(l)
    return () => {
      this.listeners.delete(l)
    }
  }
  get = () => this.state
  setKeyHandler(fn: ((e: KeyboardEvent) => boolean) | null) {
    this.keyHandler = fn
  }
  set(next: Partial<SlashState>) {
    this.state = { ...this.state, ...next }
    this.listeners.forEach((l) => l())
  }
}

export function createSlashCommand(store: SlashStore, getItems: () => SlashItem[]) {
  const key = new PluginKey("reportSlash")
  return Extension.create({
    name: "reportSlash",
    addProseMirrorPlugins() {
      return [
        Suggestion<SlashItem, SlashItem>({
          editor: this.editor,
          pluginKey: key,
          char: "/",
          allow: ({ state, range }) => state.doc.resolve(range.from).parent.type.name !== "title",
          items: ({ query }) => matchSlashItems(getItems(), query),
          command: ({ editor, range, props }) => props.run(editor, range),
          render: () => {
            const push = (props: SuggestionProps<SlashItem, SlashItem>) =>
              store.set({
                open: true,
                items: props.items,
                query: props.query,
                rect: props.clientRect?.() ?? null,
                command: (item) => {
                  if (!item.disabled) props.command(item)
                },
              })
            return {
              onStart: push,
              onUpdate: push,
              onExit: () => store.set({ open: false, command: null }),
              onKeyDown: ({ event }: SuggestionKeyDownProps) => {
                if (event.key === "Escape") {
                  store.set({ open: false })
                  return true
                }
                return store.keyHandler?.(event) ?? false
              },
            }
          },
        }),
      ]
    },
  })
}

/* ------------------------------------------------------------------ */
/* Dragging                                                            */
/* ------------------------------------------------------------------ */

/**
 * Start a move: ProseMirror moves what `view.dragging` names when the drop
 * lands in the editor (its own drop handler), so the ⋮⋮ handle, which lives
 * outside the editor's DOM, sets it the way a drag from inside would.
 */
export function startBlockDrag(view: EditorView, sel: NodeSelection) {
  ;(view as unknown as { dragging: unknown }).dragging = { slice: sel.content(), move: true, node: sel }
}

export function endBlockDrag(view: EditorView) {
  window.setTimeout(() => {
    ;(view as unknown as { dragging: unknown }).dragging = null
  }, 50)
}

/** The drag data type of a chart taken from the Charts tab. */
export const CHART_MIME = "application/x-rantai-report-chart"

/** A chart dragged from the Charts tab lands at the gap between blocks nearest the pointer. */
export function createShelfDrop(onDrop: (chartId: string, pos: number) => void) {
  return Extension.create({
    name: "shelfDrop",
    addProseMirrorPlugins() {
      return [
        new Plugin({
          key: new PluginKey("shelfDrop"),
          props: {
            handleDrop(view, event) {
              const id = event.dataTransfer?.getData(CHART_MIME)
              if (!id) return false
              const at = view.posAtCoords({ left: event.clientX, top: event.clientY })
              if (!at) return false
              const $pos = view.state.doc.resolve(at.pos)
              let pos: number
              if ($pos.depth === 0) pos = at.pos
              else {
                const dom = view.nodeDOM($pos.before(1)) as HTMLElement | null
                const rect = dom?.getBoundingClientRect()
                pos = rect && event.clientY > rect.top + rect.height / 2 ? $pos.after(1) : $pos.before(1)
              }
              // Nothing goes above the title.
              pos = Math.max(pos, view.state.doc.child(0).nodeSize)
              event.preventDefault()
              onDrop(id, pos)
              return true
            },
          },
        }),
      ]
    },
  })
}

/* ------------------------------------------------------------------ */
/* The assistant's draft (BI-62 PR B, D8)                              */
/* ------------------------------------------------------------------ */

/**
 * The draft node's schema alone (the page adds its React view; tests use
 * this). It holds only which draft (`draft`, the key of its answer in the
 * page's draft store), the chart it describes, and whether it took the place
 * of an empty line (`replaced`), so taking it out can put that line back.
 * The answer itself never enters the document: streaming text is not a
 * transaction, and `docToBlocks` skips the node, so autosave never sees a
 * draft (D8: drafts never save until accepted).
 */
export const DRAFT_NODE_SPEC = {
  name: DRAFT_NODE,
  group: "block",
  atom: true,
  selectable: false,
  draggable: false,
  addAttributes() {
    return {
      draft: { default: null, rendered: false },
      chart: { default: null, rendered: false },
      replaced: { default: false, rendered: false },
    }
  },
  // Never read back from HTML: a pasted copy of a draft is not a draft (its answer lives in the page).
  parseHTML: () => [],
  renderHTML: () => ["div", { "data-ai-draft": "" }] as const,
}

export const PlainDraftNode = Node.create(DRAFT_NODE_SPEC)

/** Where the draft `id` is in the document, if it still is. */
export function findDraft(doc: PmNode, id: string): { pos: number; node: PmNode } | null {
  let pos = 0
  for (let i = 0; i < doc.childCount; i++) {
    const node = doc.child(i)
    if (node.type.name === DRAFT_NODE && node.attrs.draft === id) return { pos, node }
    pos += node.nodeSize
  }
  return null
}

/** The drafts the document holds, by id. */
export function draftIds(doc: PmNode): Set<string> {
  const ids = new Set<string>()
  doc.forEach((node) => {
    if (node.type.name === DRAFT_NODE && typeof node.attrs.draft === "string") ids.add(node.attrs.draft)
  })
  return ids
}

/**
 * Put a draft where `insertBlock` would put a block: at `at`, in an empty
 * line's place (remembered, so Discard puts the line back), or under the
 * block holding the cursor. Outside the undo history: a draft is not an edit
 * of the report, so Undo never steps through it.
 */
export function insertDraft(editor: Editor, attrs: { draft: string; chart?: string | null }, at?: number): void {
  const { state } = editor
  const titleEnd = state.doc.child(0).nodeSize
  let from: number
  let to: number
  let replaced = false
  if (typeof at === "number") {
    from = to = Math.max(at, titleEnd)
  } else {
    const { selection } = state
    const { $from } = selection
    if ($from.depth === 0) {
      from = to = Math.max(selection instanceof NodeSelection ? selection.to : $from.pos, titleEnd)
    } else {
      const top = $from.node(1)
      if (top.type.name === "paragraph" && top.content.size === 0) {
        from = $from.before(1)
        to = $from.after(1)
        replaced = true
      } else {
        from = to = Math.max($from.after(1), titleEnd)
      }
    }
  }
  const node = state.schema.nodes[DRAFT_NODE].create({ draft: attrs.draft, chart: attrs.chart ?? null, replaced })
  const tr = state.tr.replaceWith(from, to, node)
  tr.setMeta("addToHistory", false)
  editor.view.dispatch(tr)
}

/**
 * Take the draft out, putting back the empty line it took the place of, so
 * the document is as it was before the draft (Discard). Outside the undo
 * history. Answers whether an empty line now stands where the draft was, or
 * null when the draft is gone already.
 */
export function removeDraft(editor: Editor, id: string): { pos: number; line: boolean } | null {
  const { state } = editor
  const found = findDraft(state.doc, id)
  if (!found) return null
  const { pos, node } = found
  // The schema is `title block+`: the page keeps a line to write on.
  const line = node.attrs.replaced === true || state.doc.childCount <= 2
  const tr = line
    ? state.tr.replaceWith(pos, pos + node.nodeSize, state.schema.nodes.paragraph.create())
    : state.tr.delete(pos, pos + node.nodeSize)
  tr.setMeta("addToHistory", false)
  editor.view.dispatch(tr)
  return { pos, line }
}

/**
 * Accept: the draft becomes `blocks` where it stood. The draft is first
 * taken out as Discard would (outside history), then the blocks go in as one
 * history step of their own, so a single Undo takes the accepted text out
 * and leaves the page as it was before the draft. Answers false when the
 * draft is gone.
 */
export function acceptDraft(editor: Editor, id: string, blocks: readonly ReportBlock[]): boolean {
  const removed = removeDraft(editor, id)
  if (!removed) return false
  if (blocks.length === 0) return true
  const content = (blocksToDoc("", blocks).content ?? []).slice(1) as JSONContent[]
  const { pos, line } = removed
  editor
    .chain()
    .command(({ tr }) => {
      closeHistory(tr)
      return true
    })
    .insertContentAt(line ? { from: pos, to: pos + 2 } : pos, content)
    .scrollIntoView()
    .run()
  return true
}

/* ------------------------------------------------------------------ */
/* The whole set                                                       */
/* ------------------------------------------------------------------ */

/** The chart node's schema alone (the page adds its React view; tests use this). */
export const CHART_NODE_SPEC = {
  name: "chart",
  group: "block",
  atom: true,
  draggable: true,
  selectable: true,
  addAttributes() {
    return {
      chart: { default: null, parseHTML: (el: HTMLElement) => el.getAttribute("data-chart"), renderHTML: (a: { chart?: unknown }) => ({ "data-chart": a.chart }) },
      width: { default: "text", parseHTML: (el: HTMLElement) => el.getAttribute("data-width") ?? "text", renderHTML: (a: { width?: unknown }) => ({ "data-width": a.width }) },
    }
  },
  parseHTML: () => [{ tag: "div[data-report-chart-node]" }],
  renderHTML: ({ HTMLAttributes }: { HTMLAttributes: Record<string, unknown> }) => ["div", mergeAttributes(HTMLAttributes, { "data-report-chart-node": "" })] as const,
}

export const PlainChartNode = Node.create(CHART_NODE_SPEC)

/**
 * The editor's extensions. `chartNode` and `draftNode` are the chart and
 * draft nodes with their React views on the page; `slash` and `onShelfDrop`
 * are left out where nothing reads them.
 */
export function reportExtensions({
  chartNode = PlainChartNode,
  draftNode = PlainDraftNode,
  slash,
  onShelfDrop,
  onLinkKey,
}: {
  chartNode?: AnyExtension
  draftNode?: AnyExtension
  slash?: { store: SlashStore; items: () => SlashItem[] }
  onShelfDrop?: (chartId: string, pos: number) => void
  /** ⌘K with words selected: the selection toolbar opens its link field. */
  onLinkKey?: () => void
} = {}): AnyExtension[] {
  const LinkKey = Extension.create({
    name: "linkKey",
    addKeyboardShortcuts: () => ({
      "Mod-k": ({ editor }) => {
        if (editor.state.selection.empty || !onLinkKey) return false
        onLinkKey()
        return true
      },
    }),
  })
  return [
    ReportDocument,
    Title,
    ReportHeading,
    StarterKit.configure({
      document: false,
      heading: false,
      code: false,
      codeBlock: false,
      strike: false,
      underline: false,
      dropcursor: { class: "rd-dropcursor", width: 3 },
      link: {
        openOnClick: false,
        autolink: true,
        defaultProtocol: "https",
        // The server's rule (plan D3): a pasted or typed `javascript:` link is never made.
        isAllowedUri: (url) => safeReportUrl(url),
        HTMLAttributes: { rel: "noopener noreferrer", target: null },
      },
    }),
    Typography,
    Placeholder.configure({
      includeChildren: false,
      showOnlyCurrent: false,
      // Which empty line shows its hint is decided in CSS (`report-document.css`):
      // the editor's state is not the new one yet while this runs.
      placeholder: ({ node }) => {
        if (node.type.name === "title") return "Untitled report"
        if (node.type.name === "heading") return node.attrs.level === 1 ? "Heading 1" : "Heading 2"
        return node.type.name === "paragraph" ? "Type / for charts, headings and lists" : ""
      },
    }),
    Focus.configure({ className: "has-focus", mode: "shallowest" }),
    MoveBlock,
    ClickBack,
    LinkKey,
    chartNode,
    draftNode,
    ...(slash ? [createSlashCommand(slash.store, slash.items)] : []),
    ...(onShelfDrop ? [createShelfDrop(onShelfDrop)] : []),
  ]
}
