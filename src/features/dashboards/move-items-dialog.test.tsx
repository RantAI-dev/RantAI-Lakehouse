import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"

import { chooseOption } from "@/components/patterns/select-field.testing"
import { dashboardService } from "@/services"
import { boardItem } from "./library-items"
import { MoveItemsDialog } from "./move-items-dialog"

afterEach(() => {
  cleanup()
  mock.restore()
})

function boards(count: number) {
  return Array.from({ length: count }, (_, i) =>
    boardItem({ id: `b${i + 1}`, name: `Board ${i + 1}`, folderId: "f1" }),
  )
}

const folders = [{ id: "f1", name: "Sales", parentId: "" }]

// PR #530 review SHOULD-FIX: previewMove runs with bounded concurrency and
// reports how many dashboards could not be checked rather than dropping errors.
describe("MoveItemsDialog preview checks", () => {
  it("caps concurrent previewMove calls and reports failed checks", async () => {
    let active = 0
    let maxActive = 0
    const pending: Array<() => void> = []

    const spy = spyOn(dashboardService, "previewMove").mockImplementation(async (boardId) => {
      active += 1
      maxActive = Math.max(maxActive, active)
      await new Promise<void>((resolve) => {
        pending.push(resolve)
      })
      active -= 1
      if (boardId === "b2" || boardId === "b5") {
        throw new Error("network error")
      }
      if (boardId === "b1") {
        return { gains: [{ kind: "user", id: "u2", name: "Bima" }], loses: [], startingPointGains: false, startingPointLoses: false }
      }
      return { gains: [], loses: [], startingPointGains: false, startingPointLoses: false }
    })

    render(
      <MoveItemsDialog
        items={boards(7)}
        folders={folders}
        onOpenChange={() => {}}
        onDone={() => {}}
      />,
    )

    // First wave starts at most 4 in parallel while 7 boards need checking.
    await waitFor(() => expect(spy).toHaveBeenCalledTimes(4))
    expect(maxActive).toBeLessThanOrEqual(4)

    // Drain remaining waves until all 7 previews complete.
    while (spy.mock.calls.length < 7 || pending.length > 0) {
      const batch = pending.splice(0, pending.length)
      for (const release of batch) release()
      await new Promise((r) => setTimeout(r, 0))
    }

    expect(spy).toHaveBeenCalledTimes(7)
    expect(maxActive).toBe(4)
    expect(await screen.findByText("Access change could not be checked for 2 dashboards.")).toBeTruthy()
    expect(screen.getByText(/Will gain access: Bima\./)).toBeTruthy()
  })

  it("uses singular grammar when one dashboard preview fails", async () => {
    spyOn(dashboardService, "previewMove").mockRejectedValue(new Error("boom"))

    render(
      <MoveItemsDialog
        items={boards(1)}
        folders={folders}
        onOpenChange={() => {}}
        onDone={() => {}}
      />,
    )

    chooseOption(screen.getByRole("combobox", { name: "Folder" }), "Top level (no folder)")
    expect(await screen.findByText("Access change could not be checked for 1 dashboard.")).toBeTruthy()
  })
})
