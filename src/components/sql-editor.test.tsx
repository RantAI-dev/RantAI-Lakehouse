import { cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import type { EditorView } from "@codemirror/view"
import { SqlEditor } from "./sql-editor"

afterEach(cleanup)

describe("SqlEditor", () => {
  it("memberi tampilan editor kepada pemanggil", () => {
    const views: EditorView[] = []

    render(
      <SqlEditor
        value="SELECT 1"
        onChange={() => {}}
        onView={(view) => views.push(view)}
      />
    )

    expect(views.length).toBe(1)
    expect(views[0]?.state.doc.toString()).toBe("SELECT 1")
  })

  it("menampilkan teksnya tanpa properti baru", () => {
    const { container } = render(<SqlEditor value="SELECT 1" onChange={() => {}} />)

    expect(container.textContent).toContain("SELECT 1")
  })

  // QS-11 (LH-QS-1-01), D7: the hint is a CodeMirror placeholder, not text
  // in the document that a run could send.
  it("shows a placeholder while the document stays empty", () => {
    const views: EditorView[] = []
    const { container } = render(
      <SqlEditor
        value=""
        onChange={() => {}}
        placeholder="P"
        onView={(view) => views.push(view)}
      />
    )

    expect(container.textContent).toContain("P")
    expect(views[0]?.state.doc.toString()).toBe("")
  })
})
