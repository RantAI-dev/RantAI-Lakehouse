import { afterEach, describe, expect, it } from "bun:test"
import { Editor } from "@tiptap/core"

import { SLASH_ITEMS, blocksToDoc, docToBlocks, type DocNode } from "@/lib/report-doc"
import { moveBlock, reportExtensions } from "./report-editor-extensions"

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
