import { afterEach, describe, expect, it } from "bun:test"
import { Editor } from "@tiptap/core"

import { SLASH_ITEMS, blocksToDoc, docToBlocks, type DocNode } from "@/lib/report-doc"
import { acceptDraft, draftIds, insertDraft, moveBlock, removeDraft, reportExtensions } from "./report-editor-extensions"

// BI-62 (T4): the editor's Markdown shortcuts and Alt+↑/↓, on a live Tiptap
// editor in this file's DOM (the chart's React view is left out: the schema
// node stands in for it). Each editor is destroyed after its test.
let editors: Editor[] = []
afterEach(() => {
  for (const e of editors) e.destroy()
  editors = []
})

function editorWith(doc: DocNode = blocksToDoc("Title", [])): Editor {
  const element = document.createElement("div")
  document.body.appendChild(element)
  const editor = new Editor({ element, extensions: reportExtensions(), content: doc })
  editors.push(editor)
  return editor
}

/** Type `text` at the cursor one character at a time, as a keyboard would (input rules run). */
function type(editor: Editor, text: string) {
  const { view } = editor
  for (const ch of text) {
    const { from, to } = view.state.selection
    const handled = view.someProp("handleTextInput", (f) => f(view, from, to, ch, () => view.state.tr.insertText(ch, from, to)))
    if (!handled) view.dispatch(view.state.tr.insertText(ch, from, to))
  }
}

/** A fresh editor with the cursor on the empty line under the title. */
function onFirstLine(): Editor {
  const editor = editorWith()
  editor.commands.setTextSelection(editor.state.doc.child(0).nodeSize + 1)
  return editor
}

const blocks = (editor: Editor) => docToBlocks(editor.getJSON() as DocNode).blocks

describe("Markdown shortcuts", () => {
  const cases: [string, string, unknown][] = [
    ["# ", "Arrivals", { type: "heading", text: "Arrivals", level: 1 }],
    ["## ", "By month", { type: "heading", text: "By month", level: 2 }],
    ["- ", "North", { type: "list", items: ["North"] }],
    ["1. ", "First", { type: "list", items: ["First"], ordered: true }],
    ["> ", "Quoted", { type: "quote", text: "Quoted" }],
  ]
  for (const [shortcut, words, expected] of cases) {
    it(`"${shortcut}" makes its block`, () => {
      const editor = onFirstLine()
      type(editor, shortcut + words)
      expect(blocks(editor)).toEqual([expected as never])
    })
  }

  it('"---" makes a divider', () => {
    const editor = onFirstLine()
    type(editor, "---")
    expect(blocks(editor)).toEqual([{ type: "divider" }])
  })

  it('"### " is not a third heading level: the editor writes 1 and 2', () => {
    const editor = onFirstLine()
    type(editor, "### x")
    expect(blocks(editor)[0]).toEqual({ type: "paragraph", text: "### x" })
  })

  it("every shortcut the / menu shows is one the editor has", () => {
    const hints = SLASH_ITEMS.flatMap((i) => (i.hint ? [i.hint] : []))
    expect(hints).toEqual(["#", "##", "-", "1.", ">", "---"])
  })

  it("bold and italic typed as Markdown become marks, saved back as the subset", () => {
    const editor = onFirstLine()
    type(editor, "a **b** and *c* ")
    expect(blocks(editor)).toEqual([{ type: "paragraph", text: "a **b** and *c* " }])
    const marks = (editor.getJSON().content?.[1]?.content ?? []).map((n) => (n.marks ?? []).map((m) => m.type).join("+"))
    expect(marks).toEqual(["", "bold", "", "italic", ""])
  })

  it("a link the server would refuse is never made, typed or set", () => {
    const editor = onFirstLine()
    type(editor, "click")
    const at = editor.state.doc.child(0).nodeSize + 1
    editor.commands.setTextSelection({ from: at, to: at + 5 })
    editor.commands.setLink({ href: "javascript:alert(1)" })
    expect(blocks(editor)).toEqual([{ type: "paragraph", text: "click" }])
    editor.commands.setLink({ href: "https://a.example" })
    expect(blocks(editor)).toEqual([{ type: "paragraph", text: "[click](https://a.example)" }])
  })
})

describe("Alt+↑/↓", () => {
  const start: Parameters<typeof blocksToDoc>[1] = [
    { type: "paragraph", text: "one" },
    { type: "chart", chart: "c_1" },
    { type: "paragraph", text: "three" },
  ]

  it("moves the block with the cursor one place, and never above the title", () => {
    const editor = editorWith(blocksToDoc("T", start))
    const third = editor.state.doc.child(0).nodeSize + editor.state.doc.child(1).nodeSize + editor.state.doc.child(2).nodeSize + 1
    editor.commands.setTextSelection(third)
    expect(moveBlock(editor, -1)).toBe(true)
    expect(blocks(editor).map((b) => (b.type === "paragraph" ? b.text : b.type))).toEqual(["one", "three", "chart"])
    moveBlock(editor, -1)
    moveBlock(editor, -1)
    expect(blocks(editor).map((b) => (b.type === "paragraph" ? b.text : b.type))).toEqual(["three", "one", "chart"])
    expect(editor.state.doc.child(0).type.name).toBe("title")
  })

  it("does nothing in the title", () => {
    const editor = editorWith(blocksToDoc("T", start))
    editor.commands.setTextSelection(1)
    expect(moveBlock(editor, 1)).toBe(false)
  })
})

describe("the assistant's draft (BI-62 PR B, D8)", () => {
  const start: Parameters<typeof blocksToDoc>[1] = [
    { type: "paragraph", text: "one" },
    { type: "chart", chart: "c_1" },
  ]
  const json = (editor: Editor) => JSON.stringify(editor.getJSON())
  /** Position of the top-level block `index` (0 is the title). */
  const at = (editor: Editor, index: number) => {
    let pos = 0
    for (let i = 0; i < index; i++) pos += editor.state.doc.child(i).nodeSize
    return pos
  }

  it("takes an empty line's place and Discard puts the line back: the document is as before", () => {
    const editor = onFirstLine()
    const before = json(editor)
    insertDraft(editor, { draft: "d1" })
    expect(editor.state.doc.child(1).type.name).toBe("aiDraft")
    expect(editor.state.doc.childCount).toBe(2)
    expect(draftIds(editor.state.doc)).toEqual(new Set(["d1"]))
    // Never part of the report: nothing to save.
    expect(blocks(editor)).toEqual([])
    expect(removeDraft(editor, "d1")).toEqual({ pos: at(editor, 1), line: true })
    expect(json(editor)).toBe(before)
    expect(removeDraft(editor, "d1")).toBeNull()
  })

  it("goes under a chart and Discard leaves the document byte-identical", () => {
    const editor = editorWith(blocksToDoc("T", start))
    const before = json(editor)
    insertDraft(editor, { draft: "d2", chart: "c_1" }, at(editor, 3))
    expect(editor.state.doc.child(3).attrs).toEqual({ draft: "d2", chart: "c_1", replaced: false })
    expect(blocks(editor)).toEqual([...start])
    removeDraft(editor, "d2")
    expect(json(editor)).toBe(before)
  })

  it("is not an undo step: Undo after a draft undoes the person's last edit, not the draft", () => {
    const editor = onFirstLine()
    type(editor, "x")
    insertDraft(editor, { draft: "d3" }, at(editor, 2))
    editor.commands.undo()
    expect(blocks(editor)).toEqual([])
    expect(draftIds(editor.state.doc)).toEqual(new Set(["d3"]))
  })

  it("Accept puts the blocks where the draft was, as one undo step", () => {
    const editor = editorWith(blocksToDoc("T", start))
    const before = json(editor)
    insertDraft(editor, { draft: "d4", chart: "c_1" }, at(editor, 3))
    const accepted = [
      { type: "paragraph", text: "Visits **rose**." },
      { type: "list", items: ["a", "b"] },
    ] as const
    expect(acceptDraft(editor, "d4", accepted as never)).toBe(true)
    expect(blocks(editor)).toEqual([...start, ...accepted] as never)
    expect(draftIds(editor.state.doc).size).toBe(0)
    editor.commands.undo()
    expect(json(editor)).toBe(before)
  })

  it("Accept in an empty line's place uses that line, and Undo brings the line back", () => {
    const editor = onFirstLine()
    type(editor, "x")
    editor.commands.enter()
    const before = json(editor)
    insertDraft(editor, { draft: "d5" })
    acceptDraft(editor, "d5", [{ type: "heading", text: "Hi", level: 2 }])
    expect(blocks(editor)).toEqual([
      { type: "paragraph", text: "x" },
      { type: "heading", text: "Hi", level: 2 },
    ])
    expect(editor.state.doc.childCount).toBe(3)
    editor.commands.undo()
    expect(json(editor)).toBe(before)
  })

  it("a draft is never read back from pasted HTML", () => {
    const editor = onFirstLine()
    editor.commands.insertContent('<div data-ai-draft=""></div><p>kept</p>')
    expect(draftIds(editor.state.doc).size).toBe(0)
    expect(blocks(editor)).toEqual([{ type: "paragraph", text: "kept" }])
  })
})
