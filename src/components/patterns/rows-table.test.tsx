import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { RowsTable } from "./rows-table"

afterEach(cleanup)

const rows = [{ a: null, n: 1 }, { a: "", n: 2 }, { a: "Bali", n: 3 }]

// QA LH-BI-6-38 (BI-55, D8): the Rows tables name a missing and an empty
// value as the chart does, in muted text.
describe("RowsTable", () => {
  it("names a missing value (No value) and an empty one (Empty) when asked", () => {
    render(<RowsTable columns={["a", "n"]} rows={rows} namePlaceholders />)
    const missing = screen.getByText("(No value)")
    const empty = screen.getByText("(Empty)")
    expect(missing.className).toContain("text-muted-foreground")
    expect(empty.className).toContain("text-muted-foreground")
  })

  it("leaves both blank when not asked", () => {
    render(<RowsTable columns={["a", "n"]} rows={rows} />)
    expect(screen.queryByText("(No value)")).toBeNull()
    expect(screen.queryByText("(Empty)")).toBeNull()
    expect(screen.getByText("Bali")).toBeTruthy()
  })
})
