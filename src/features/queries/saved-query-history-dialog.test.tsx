// The history of a saved query (`QS-3` C2, C6): the versions the server
// keeps, newest first, and a way back to one of them.
//
// The label of a version is the server's own — "Restored version 2" is
// composed by the store, not by the console — so a test reads it from the
// data and never from a string the client built.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { queryService } from "@/services"
import type { SavedQuery, SavedQueryVersion } from "@/services/contracts/queries"
import { ServiceError } from "@/services/errors"
import { SavedQueryHistoryDialog } from "./saved-query-history-dialog"

afterEach(() => {
  cleanup()
  mock.restore()
})

/** The versions the server sent: newest first, as it orders them. */
const VERSIONS: SavedQueryVersion[] = [
  {
    version: 3,
    title: "Revenue",
    sql: "SELECT revenue FROM daily",
    authorId: "u1",
    authorName: "Rina Wijaya",
    label: "",
    createdAt: "2026-11-01T09:00:00.000Z",
  },
  {
    version: 2,
    title: "Revenue by month",
    sql: "SELECT revenue FROM monthly",
    authorId: "u1",
    authorName: "Rina Wijaya",
    label: "",
    createdAt: "2026-10-31T09:00:00.000Z",
  },
  {
    version: 1,
    title: "Revenue",
    sql: "SELECT revenue FROM weekly",
    authorId: "u1",
    authorName: "Rina Wijaya",
    label: "",
    createdAt: "2026-10-30T09:00:00.000Z",
  },
]

/**
 * The versions the server sends after a revert: a new version that carries
 * the content of the version it went back to, and the server's own label
 * naming **that** version — not the new one.
 */
const AFTER_REVERT: SavedQueryVersion[] = [
  {
    version: 4,
    title: "Revenue by month",
    sql: "SELECT revenue FROM monthly",
    authorId: "u1",
    authorName: "Rina Wijaya",
    label: "Restored version 2",
    createdAt: "2026-11-02T09:00:00.000Z",
  },
  ...VERSIONS,
]

function saved(over: Partial<SavedQuery> = {}): SavedQuery {
  return {
    id: "q1",
    title: "Revenue",
    sql: "SELECT 1",
    owner: "Rina Wijaya",
    updatedAt: "2026-11-01T09:00:00.000Z",
    tags: [],
    folderId: null,
    ownerId: "u1",
    level: "view",
    ...over,
  }
}

/** The server's list: the first read, then the one a revert brings. */
function stubVersions(after: SavedQueryVersion[] = AFTER_REVERT) {
  let reads = 0
  return spyOn(queryService, "listSavedVersions").mockImplementation(async () => {
    reads += 1
    return reads === 1 ? VERSIONS : after
  })
}

function renderDialog(level = "edit", onReverted = () => {}) {
  return render(
    <SavedQueryHistoryDialog
      queryId="q1"
      title="Revenue"
      level={level}
      onClose={() => {}}
      onReverted={onReverted}
    />,
  )
}

describe("the history of a saved query", () => {
  it("lists the versions the server sent, newest first, with its own fields", async () => {
    stubVersions()
    renderDialog("view")

    await screen.findByText("SELECT revenue FROM daily")

    // The order is the server's (`ORDER BY version DESC`): the console does
    // not sort them again.
    expect(screen.getAllByText(/^v\d+$/).map((node) => node.textContent)).toEqual([
      "v3",
      "v2",
      "v1",
    ])
    expect(screen.getByText("Revenue by month")).toBeDefined()
    expect(screen.getByText("SELECT revenue FROM monthly")).toBeDefined()
    expect(screen.getAllByText("Rina Wijaya").length).toBe(3)
  })

  it("asks before it goes back to a version", async () => {
    stubVersions()
    const revert = spyOn(queryService, "revertSavedVersion").mockResolvedValue(saved())
    renderDialog("edit")

    await screen.findByText("SELECT revenue FROM monthly")
    fireEvent.click(screen.getAllByRole("button", { name: "Revert" })[0])

    // Nothing is written before the person says yes.
    expect(revert.mock.calls.length).toBe(0)
    expect(await screen.findByText("Revert to version 2")).toBeDefined()
  })

  it("goes back to the version the person confirmed", async () => {
    stubVersions()
    const revert = spyOn(queryService, "revertSavedVersion").mockResolvedValue(saved())
    const onReverted = mock(() => {})
    renderDialog("edit", onReverted)

    await screen.findByText("SELECT revenue FROM monthly")
    fireEvent.click(screen.getAllByRole("button", { name: "Revert" })[0])
    fireEvent.click(await screen.findByRole("button", { name: "Go back" }))

    await waitFor(() => {
      expect(revert.mock.calls.at(-1)?.[0]).toBe("q1")
      expect(revert.mock.calls.at(-1)?.[1]).toBe(2)
    })
    expect(onReverted).toHaveBeenCalled()
  })

  it("shows the label the server wrote, and writes none of its own", async () => {
    stubVersions()
    spyOn(queryService, "revertSavedVersion").mockResolvedValue(saved())
    renderDialog("edit")

    await screen.findByText("SELECT revenue FROM monthly")
    fireEvent.click(screen.getAllByRole("button", { name: "Revert" })[0])
    fireEvent.click(await screen.findByRole("button", { name: "Go back" }))

    // The server named the version it went back to, not the version it
    // wrote: "Restored version 2", on version 4.
    expect(await screen.findByText("Restored version 2")).toBeDefined()
    // The label belongs to the version the server wrote (v4), not to the
    // version it went back to (v2): the console puts the label where the
    // server put it.
    const rows = screen.getAllByRole("listitem")
    const rowOf = (version: string) =>
      rows.find((row) =>
        Array.from(row.querySelectorAll("span")).some(
          (node) => node.textContent === version,
        ),
      )
    expect(rowOf("v4")?.textContent).toContain("Restored version 2")
    expect(rowOf("v2")?.textContent).not.toContain("Restored version")
  })

  it("offers no way back to a caller who may only look", async () => {
    stubVersions()
    renderDialog("view")

    await screen.findByText("SELECT revenue FROM monthly")

    expect(screen.queryAllByRole("button", { name: "Revert" }).length).toBe(0)
  })

  it("shows the server's sentence when the revert is refused", async () => {
    stubVersions()
    spyOn(queryService, "revertSavedVersion").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "permission_denied: You do not have permission to change this saved query.",
      ),
    )
    renderDialog("edit")

    await screen.findByText("SELECT revenue FROM monthly")
    fireEvent.click(screen.getAllByRole("button", { name: "Revert" })[0])
    fireEvent.click(await screen.findByRole("button", { name: "Go back" }))

    expect((await screen.findByRole("alert")).textContent).toBe(
      "You do not have permission to change this saved query.",
    )
  })
})
