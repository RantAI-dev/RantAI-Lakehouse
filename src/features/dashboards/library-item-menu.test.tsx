import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"

import { boardItem, reportItem, templateItem } from "./library-items"
import { BUILTIN_NOTE, ItemMenu, type ItemActions } from "./library-menus"

afterEach(cleanup)

const board = boardItem({ id: "b1", name: "Sales" })
const main = boardItem({ id: "default", name: "Main", builtin: true })
const report = reportItem({
  id: "r1", title: "Wisman", folderId: "", createdBy: "Rina", updatedBy: "Rina",
  updatedAt: "2026-11-13T00:00:00Z", blockCount: 3, chartCount: 1,
})
const template = templateItem({
  id: "t1", name: "Sales overview", sourceBoard: "b_src", sourceBoardName: "Quarterly sales",
  chartCount: 3, createdBy: "Rina", createdAt: "2026-11-13T00:00:00Z",
})
const noop = () => {}
const reader: ItemActions = { isFavourite: () => false, onToggleFavourite: noop }
const writer: ItemActions = {
  ...reader, onRename: noop, onMove: noop, onDuplicate: noop, onTrash: noop, onUseTemplate: noop, onDeleteTemplate: noop,
}

function open(item: Parameters<typeof ItemMenu>[0]["item"], actions: ItemActions) {
  render(<ItemMenu item={item} actions={actions} />)
  fireEvent.click(screen.getByRole("button", { name: `Actions for ${item.name}` }))
}

// BI-18 part C review fix (SHOULD-FIX) R27: the list offers no write action without dashboard:write.
describe("the dashboard list's ⋯ menu", () => {
  it("offers Rename, Move, Duplicate and Move to trash to an editor", async () => {
    open(board, writer)
    for (const label of ["Rename…", "Move to folder…", "Duplicate", "Move to trash"]) expect(await screen.findByText(label)).toBeTruthy()
  })

  it("offers none of them to a viewer, only the favourite", async () => {
    open(board, reader)
    expect(await screen.findByText("Add to favourites")).toBeTruthy()
    for (const label of ["Rename…", "Move to folder…", "Duplicate", "Move to trash"]) expect(screen.queryByText(label)).toBeNull()
  })

  // BI-59: the built-in board gets the same menu in the same place, its
  // impossible entries disabled with the reason.
  it("gives the built-in Main a menu: Duplicate works, Rename, Move and trash are disabled with the reason", async () => {
    open(main, writer)
    expect(await screen.findByText("Duplicate")).toBeTruthy()
    for (const label of ["Rename…", "Move to folder…", "Move to trash"]) {
      expect(screen.getByText(label).closest("[role=menuitem]")?.getAttribute("aria-disabled")).toBe("true")
    }
    expect(screen.getByText(BUILTIN_NOTE)).toBeTruthy()
  })

  it("offers a report Open, Copy link, Edit, Move and trash, and no favourite", async () => {
    open(report, writer)
    for (const label of ["Open", "Copy link", "Edit", "Move to folder…", "Move to trash"]) expect(await screen.findByText(label)).toBeTruthy()
    expect(screen.queryByText("Add to favourites")).toBeNull()
  })

  it("offers a template Use template and Delete template", async () => {
    open(template, writer)
    expect(await screen.findByText("Use template…")).toBeTruthy()
    expect(screen.getByText("Delete template…")).toBeTruthy()
  })
})
