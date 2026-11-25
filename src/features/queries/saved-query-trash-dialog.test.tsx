// The trash of the saved queries (`QS-3` C2, C6): what the caller may edit
// waits there with the days it has left, and a Restore brings it back.
//
// The server sends only the queries the caller may edit, so the console
// lists what it gets and does not filter it again. The service is spied;
// nothing leaves the test.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { queryService } from "@/services"
import type { QueryFolder, SavedQuery, SavedQueryTrashEntry } from "@/services/contracts/queries"
import { ServiceError } from "@/services/errors"
import { SavedQueryTrashDialog } from "./saved-query-trash-dialog"

afterEach(() => {
  cleanup()
  mock.restore()
})

const FOLDERS: QueryFolder[] = [
  {
    id: "personal-1",
    parentId: null,
    name: "Personal space of Rina Wijaya",
    isPersonal: true,
    ownerId: "u1",
    level: "manage",
  },
  {
    id: "team",
    parentId: null,
    name: "Team",
    isPersonal: false,
    ownerId: "u2",
    level: "manage",
  },
]

/** Two queries waiting in the trash: one in a folder, one in the shared space. */
const TRASH: SavedQueryTrashEntry[] = [
  {
    id: "q1",
    title: "Old revenue",
    folderId: "team",
    deletedAt: "2026-11-01T09:00:00.000Z",
    daysLeft: 12,
  },
  {
    id: "q2",
    title: "Lost costs",
    folderId: null,
    deletedAt: "2026-10-30T09:00:00.000Z",
    daysLeft: 3,
  },
]

function stubTrash(entries: SavedQueryTrashEntry[] = TRASH) {
  return spyOn(queryService, "listSavedTrash").mockResolvedValue(entries)
}

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
    parameters: [],
    ...over,
  }
}

function renderDialog(level = "edit") {
  return render(
    <SavedQueryTrashDialog
      folders={FOLDERS}
      level={level}
      onClose={() => {}}
      onRestored={() => {}}
    />,
  )
}

describe("the trash of the saved queries", () => {
  it("lists what waits there with its folder and the days it has left", async () => {
    stubTrash()
    renderDialog()

    expect(await screen.findByText("Old revenue")).toBeDefined()
    expect(screen.getByText("Team")).toBeDefined()
    expect(screen.getByText("12 days left")).toBeDefined()
    // A query in the shared space waits in the shared space.
    expect(screen.getByText("Lost costs")).toBeDefined()
    expect(screen.getByText("Shared")).toBeDefined()
    expect(screen.getByText("3 days left")).toBeDefined()
  })

  it("offers a Restore to a caller who may edit", async () => {
    stubTrash()
    renderDialog("edit")

    await screen.findByText("Old revenue")

    expect(screen.queryAllByRole("button", { name: "Restore" }).length).toBe(2)
  })

  it("offers no Restore to a caller who may only look", async () => {
    stubTrash()
    renderDialog("view")

    await screen.findByText("Old revenue")

    expect(screen.queryAllByRole("button", { name: "Restore" }).length).toBe(0)
  })

  it("brings a query back and reads the trash again", async () => {
    const list = stubTrash()
    const restore = spyOn(queryService, "restoreSaved").mockResolvedValue(saved())
    const onRestored = mock(() => {})
    render(
      <SavedQueryTrashDialog
        folders={FOLDERS}
        level="edit"
        onClose={() => {}}
        onRestored={onRestored}
      />,
    )

    fireEvent.click((await screen.findAllByRole("button", { name: "Restore" }))[0])

    await waitFor(() => {
      expect(restore.mock.calls.at(-1)?.[0]).toBe("q1")
    })
    // The list is read again, so the query that came back leaves it.
    await waitFor(() => {
      expect(list.mock.calls.length).toBeGreaterThan(1)
    })
    expect(onRestored).toHaveBeenCalled()
  })

  it("shows the server's sentence when the restore is refused", async () => {
    stubTrash()
    spyOn(queryService, "restoreSaved").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "permission_denied: You do not have permission to change this saved query.",
      ),
    )
    renderDialog("edit")

    fireEvent.click((await screen.findAllByRole("button", { name: "Restore" }))[0])

    expect((await screen.findByRole("alert")).textContent).toBe(
      "You do not have permission to change this saved query.",
    )
  })
})
