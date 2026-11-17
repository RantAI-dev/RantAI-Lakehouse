import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { TileDataDialog } from "./tile-dialogs"

afterEach(cleanup)

const cell = { columns: ["region", "n"], rows: [{ region: "north", n: 3 }] }

// BI-1 review fix (SHOULD-FIX) R6: View data's own CSV button obeys the switch too.
describe("View data", () => {
  it("offers the CSV download by default", () => {
    render(<TileDataDialog title="Visits" cell={cell} onClose={() => {}} />)
    expect(screen.getByRole("button", { name: /Download CSV/ })).toBeTruthy()
  })

  // QA LH-BI-6-38 (BI-55, D8): the numbers behind a chart name a missing and
  // an empty category as the chart does.
  it("the numbers behind a chart name a missing and an empty category", () => {
    const withBlanks = { columns: ["a", "n"], rows: [{ a: null, n: 1 }, { a: "", n: 2 }, { a: "Bali", n: 3 }] }
    render(<TileDataDialog title="Visits" cell={withBlanks} onClose={() => {}} />)
    expect(screen.getByText("(No value)")).toBeTruthy()
    expect(screen.getByText("(Empty)")).toBeTruthy()
  })

  it("hides the CSV download for a person whose every role is switched off from downloading", () => {
    render(<TileDataDialog title="Visits" cell={cell} onClose={() => {}} canDownload={false} />)
    expect(screen.queryByRole("button", { name: /Download CSV/ })).toBeNull()
    expect(screen.getByText("1 rows")).toBeTruthy()
  })
})
