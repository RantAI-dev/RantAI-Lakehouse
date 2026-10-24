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
})
