import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"

import { BoardActionsMenu, type BoardRowActions } from "./dashboard-list-columns"

afterEach(cleanup)

const board = { id: "b1", name: "Sales" }
const noop = () => {}

function open(actions: BoardRowActions) {
  render(<BoardActionsMenu board={board} actions={actions} />)
  fireEvent.click(screen.getByRole("button", { name: "Actions for Sales" }))
}

// BI-18 part C review fix (SHOULD-FIX) R27: the list offers no write action without dashboard:write.
describe("the dashboard list's ⋯ menu", () => {
  it("offers Rename, Move, Duplicate and Delete to an editor", async () => {
    open({ onRename: noop, onMove: noop, onDuplicate: noop, onDelete: noop })
    for (const label of ["Rename", "Move to folder…", "Duplicate", "Delete"]) expect(await screen.findByText(label)).toBeTruthy()
  })

  it("offers none of them to a viewer, only the favourite", async () => {
    open({ onToggleFavourite: noop, isFavourite: () => false })
    expect(await screen.findByText("Add to favourites")).toBeTruthy()
    for (const label of ["Rename", "Move to folder…", "Duplicate", "Delete"]) expect(screen.queryByText(label)).toBeNull()
  })
})
