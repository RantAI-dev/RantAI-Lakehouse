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

  it("hides the CSV download for a person whose every role is switched off from downloading", () => {
    render(<TileDataDialog title="Visits" cell={cell} onClose={() => {}} canDownload={false} />)
    expect(screen.queryByRole("button", { name: /Download CSV/ })).toBeNull()
    expect(screen.getByText("1 rows")).toBeTruthy()
  })
})
