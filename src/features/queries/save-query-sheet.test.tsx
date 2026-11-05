// The save sheet of Query Studio (`QS-3` C4): where a query lands, and
// what the sheet says when it is opened.
//
// The sheet asks the server for the folders and for the query the tab came
// from; both are stubbed here, so nothing leaves the test.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { queryService } from "@/services"
import type { QueryFolder, SaveQueryInput, SavedQuery } from "@/services/contracts/queries"
import { SaveQuerySheet } from "./save-query-sheet"

afterEach(() => {
  cleanup()
  mock.restore()
})

/** The folders the server sent: the caller's own space and two shared ones. */
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
    id: "reports",
    parentId: "personal-1",
    name: "Reports",
    isPersonal: false,
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

/** The saved query the tab came from. */
const SAVED: SavedQuery = {
  id: "q1",
  title: "Orders",
  sql: "SELECT 1",
  owner: "Rina Wijaya",
  updatedAt: "2026-11-04T10:00:00.000Z",
  tags: ["finance", "gold"],
  folderId: null,
  ownerId: "u1",
  level: "view",
}

function renderSheet(over: {
  readonly mode?: "create" | "update"
  readonly savedId?: string | null
  readonly sql?: string
} = {}) {
  const onSave = mock<(input: SaveQueryInput) => Promise<unknown>>(
    async () => ({ id: "q2" }),
  )
  render(
    <SaveQuerySheet
      open
      onOpenChange={() => {}}
      sql={over.sql ?? "SELECT 1"}
      saving={false}
      onSave={onSave}
      mode={over.mode ?? "create"}
      savedId={over.savedId ?? null}
    />,
  )
  return { onSave }
}

/** The folder the picker shows. */
async function picker() {
  return (await screen.findByLabelText("Folder")) as HTMLSelectElement
}

describe("the save sheet", () => {
  it("lists the folders and starts in the caller's own space", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    renderSheet()

    const select = await picker()
    await waitFor(() => expect(select.options.length).toBe(4))
    expect(Array.from(select.options).map((option) => option.text)).toEqual([
      "Personal space (only you)",
      "Personal space (only you) / Reports",
      "Shared",
      "Team",
    ])
    expect(select.value).toBe("personal")
  })

  it("sends no folder at all for the caller's own space", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    const { onSave } = renderSheet()

    fireEvent.click(await screen.findByRole("button", { name: "Save as new" }))

    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1))
    const input = onSave.mock.calls[0]?.[0] as SaveQueryInput
    expect(input.title).toBe("SELECT 1")
    expect(input.sql).toBe("SELECT 1")
    expect("folderId" in input).toBe(false)
  })

  it("sends the shared space as null, the way the server reads it", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    const { onSave } = renderSheet()

    const select = await picker()
    fireEvent.change(select, { target: { value: "shared" } })
    fireEvent.click(screen.getByRole("button", { name: "Save as new" }))

    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1))
    const input = onSave.mock.calls[0]?.[0] as SaveQueryInput
    expect(input.folderId).toBeNull()
  })

  it("sends a folder by its id", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    const { onSave } = renderSheet()

    const select = await picker()
    fireEvent.change(select, { target: { value: "team" } })
    fireEvent.click(screen.getByRole("button", { name: "Save as new" }))

    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1))
    const input = onSave.mock.calls[0]?.[0] as SaveQueryInput
    expect(input.folderId).toBe("team")
  })

  it("offers no folder and no shared-with-the-team line when the query is being changed", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    spyOn(queryService, "getSaved").mockResolvedValue(SAVED)
    renderSheet({ mode: "update", savedId: "q1" })

    await waitFor(() => expect(screen.queryAllByLabelText("Folder").length).toBe(0))
    expect(screen.queryAllByText(/shared with the team/i).length).toBe(0)
    expect(screen.getByRole("button", { name: "Save" })).toBeTruthy()
  })

  it("keeps the title and the tags of the saved query for a copy of it", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    spyOn(queryService, "getSaved").mockResolvedValue(SAVED)
    renderSheet({ mode: "create", savedId: "q1" })

    await waitFor(() =>
      expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("Orders")
    )
    expect((screen.getByLabelText("Tags") as HTMLInputElement).value).toBe("finance, gold")
  })

  it("starts in the caller's own space again when the sheet is opened anew", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    const sheet = (open: boolean) => (
      <SaveQuerySheet
        open={open}
        onOpenChange={() => {}}
        sql="SELECT 1"
        saving={false}
        onSave={async () => ({ id: "q2" })}
        mode="create"
        savedId={null}
      />
    )
    const { rerender } = render(sheet(true))

    const select = (await picker()) as HTMLSelectElement
    fireEvent.change(select, { target: { value: "team" } })
    expect(select.value).toBe("team")

    // The sheet is closed and opened again for the next query: the picker
    // starts in the caller's own space, not where the last one was left.
    rerender(sheet(false))
    rerender(sheet(true))

    const reopened = (await picker()) as HTMLSelectElement
    await waitFor(() => expect(reopened.options.length).toBe(4))
    expect(reopened.value).toBe("personal")
  })
})
