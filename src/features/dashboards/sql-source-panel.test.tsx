import { render, screen } from "@testing-library/react"
import { afterAll, describe, expect, it, mock } from "bun:test"

import * as realSqlEditor from "@/components/sql-editor"
const realEditor = { ...realSqlEditor }

// CodeMirror needs a real browser layout; the panel only needs to know that
// its label and required state reach the editor, so stub it with a textbox
// that echoes the props the panel passes through SqlEditor.
afterAll(() => {
  mock.module("@/components/sql-editor", () => realEditor)
})
// CodeMirror needs a real browser layout; the panel only needs to know that
// its label and required state reach the editor, so stub it with a textbox
// that echoes the props the panel passes through SqlEditor.
mock.module("@/components/sql-editor", () => ({
  ...realEditor,
  SqlEditor: ({ labelledBy, required }: { labelledBy?: string; required?: boolean }) => (
    <textarea aria-labelledby={labelledBy} aria-required={required} />
  ),
}))

import type { SqlSourceDraft } from "./use-sql-source-draft"
import { SqlSourcePanel } from "./sql-source-panel"

function draft(): SqlSourceDraft {
  return {
    mode: "new",
    original: null,
    name: "Visitors by region",
    nameFollowsChart: false,
    showNameError: false,
    phase: "unrun",
    changed: false,
    usage: { kind: "unknown" },
    sql: "select 1",
    setSql: mock(),
    setName: mock(),
    running: false,
    runError: null,
    result: null,
    run: mock(),
  } as unknown as SqlSourceDraft
}

describe("SQL source panel required indicators (UX-1)", () => {
  it("associates and marks both the SQL editor and the source name", () => {
    const { container } = render(<SqlSourcePanel draft={draft()} onCancelEdit={mock()} />)
    const sqlLabel = screen.getByText("SQL").closest("label")
    expect(sqlLabel?.getAttribute("data-required")).toBe("")
    // The star is CSS-only, so the visible label text stays exactly "SQL".
    expect(sqlLabel?.textContent).toBe("SQL")
    const sqlEditor = screen.getByLabelText("SQL")
    expect(sqlEditor.getAttribute("aria-required")).toBe("true")
    expect(sqlEditor.tagName).toBe("TEXTAREA")

    const nameInput = screen.getByLabelText("Source name")
    expect(nameInput.getAttribute("aria-required")).toBe("true")
    expect(container.querySelector('label[for="ch-source-name"]')?.getAttribute("data-required")).toBe("")
    // One legend for the panel, not one per field.
    expect(screen.getByText(/Required/)).toBeDefined()
    expect(container.querySelectorAll("label[data-required]").length).toBe(2)
  })
})
