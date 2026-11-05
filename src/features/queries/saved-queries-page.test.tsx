// The saved-queries console (`QS-3` C1, C2): the tree on the left, the
// search that asks the server, the folder actions, and the row actions by
// the level the caller holds. The service is spied; nothing leaves the
// test. `next/navigation` is stubbed before any import resolves
// (`mock.module` is hoisted), for the table's URL state, and
// `TableProviders` supplies the rest of it, as the root layout does.
//
// A menu item that must be absent is counted, not compared with `null`:
// bun's `toBeNull` on a live element neither fails nor returns quickly, so
// the count is the honest assertion.

mock.module("next/navigation", () => ({
  usePathname: () => "/query-studio/saved",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { queryService } from "@/services"
import type { QueryFolder, SavedQuery } from "@/services/contracts/queries"
import { ServiceError } from "@/services/errors"
import { SavedQueriesPage } from "./saved-queries-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

/**
 * The tree the server sends: the caller's own space, a shared folder and
 * one under it. The personal space is a folder of its own, so a folder in
 * it sits one level deeper than a folder in the shared space.
 */
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
  {
    id: "quarterly",
    parentId: "team",
    name: "Quarterly",
    isPersonal: false,
    ownerId: "u2",
    level: "edit",
  },
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

/**
 * The service, spied.
 *
 * `listSavedFiltered` answers what the caller asked for: the folder it
 * names, and the text it was given. The stub never decides what matches —
 * that is the server's business — so a test can see what the console does
 * with the answer.
 */
function stubServices(
  queries: SavedQuery[] = [],
  folders: QueryFolder[] = FOLDERS,
) {
  const list = spyOn(queryService, "listSavedFiltered").mockResolvedValue(queries)
  spyOn(queryService, "listFolders").mockResolvedValue(folders)
  spyOn(queryService, "getSchedulingCapability").mockResolvedValue({
    supported: false,
    reason: "not yet",
  })
  return { list }
}

function renderPage() {
  return render(
    <TableProviders>
      <SavedQueriesPage />
    </TableProviders>,
  )
}

/** How many menu items carry that name. */
function menuItems(name: string): number {
  return screen.queryAllByRole("menuitem", { name }).length
}

describe("the saved-queries console", () => {
  it("asks the server for the text typed in the search box, and does not filter in the browser", async () => {
    const list = spyOn(queryService, "listSavedFiltered").mockImplementation(
      (async (filter?: { q?: string }) =>
        filter?.q
          ? [saved({ id: "q1", title: "Zebra" }), saved({ id: "q2", title: "Sent by the server" })]
          : [saved({ id: "q1", title: "Zebra" })]) as typeof queryService.listSavedFiltered,
    )
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    spyOn(queryService, "getSchedulingCapability").mockResolvedValue({
      supported: false,
      reason: "not yet",
    })
    renderPage()
    await screen.findByText("Zebra")

    fireEvent.change(screen.getByLabelText("Search saved queries"), {
      target: { value: "revenue" },
    })

    // The text goes to the server, which owns the match (P9).
    await waitFor(
      () => {
        expect(list.mock.calls.at(-1)?.[0]?.q).toBe("revenue")
      },
      { timeout: 2000 },
    )
    // The answer that lands is the one on screen: a row the server sent
    // stays, even when its title holds none of the text typed. A filter in
    // the browser would have dropped it.
    await waitFor(
      () => {
        expect(screen.getByText("Sent by the server")).toBeDefined()
      },
      { timeout: 2000 },
    )
    expect(screen.getByText("Zebra")).toBeDefined()
  })

  it("shows the shared space, the caller's own space and the folders under each", async () => {
    stubServices()
    renderPage()

    const personal = await screen.findByRole("button", {
      name: "Personal space (only you)",
    })
    expect(personal.getAttribute("title")).toBe("Personal space of Rina Wijaya")
    expect(screen.getByRole("button", { name: "Shared" })).toBeDefined()
    expect(screen.getByRole("button", { name: "Team" })).toBeDefined()
    expect(screen.getByRole("button", { name: "Quarterly" })).toBeDefined()
  })

  it("asks for the folder the tree has selected", async () => {
    const { list } = stubServices()
    renderPage()
    await screen.findByRole("button", { name: "Shared" })

    // The shared space is the server's own word for it.
    fireEvent.click(screen.getByRole("button", { name: "Shared" }))
    await waitFor(() => {
      expect(list.mock.calls.at(-1)?.[0]?.folder).toBe("root")
    })

    // The personal space is a folder of its own, so it is named by its id.
    fireEvent.click(screen.getByRole("button", { name: "Personal space (only you)" }))
    await waitFor(() => {
      expect(list.mock.calls.at(-1)?.[0]?.folder).toBe("personal-1")
    })

    fireEvent.click(screen.getByRole("button", { name: "Quarterly" }))
    await waitFor(() => {
      expect(list.mock.calls.at(-1)?.[0]?.folder).toBe("quarterly")
    })
  })

  it("offers a viewer the copy and the duplicate, and no rename and no delete", async () => {
    stubServices([saved({ id: "q1", title: "Revenue", level: "view" })])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Revenue" }))

    expect(await screen.findByRole("menuitem", { name: "Copy SQL" })).toBeDefined()
    expect(screen.getByRole("menuitem", { name: "Copy Query ID" })).toBeDefined()
    expect(screen.getByRole("menuitem", { name: "Duplicate" })).toBeDefined()
    expect(screen.getByRole("menuitem", { name: "Open in Studio" })).toBeDefined()
    // Editing needs **edit**: the server refuses it below that level (C2).
    expect(menuItems("Rename")).toBe(0)
    expect(menuItems("Delete")).toBe(0)
  })

  it("offers an editor the rename and the delete", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "edit" })])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))

    expect(await screen.findByRole("menuitem", { name: "Rename" })).toBeDefined()
    expect(screen.getByRole("menuitem", { name: "Delete" })).toBeDefined()
  })

  it("offers a manager everything this step ships", async () => {
    stubServices([saved({ id: "q3", title: "Forecast", level: "manage" })])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Forecast" }))

    for (const label of [
      "Open in Studio",
      "Copy SQL",
      "Copy Query ID",
      "Duplicate",
      "Rename",
      "Delete",
    ]) {
      expect(await screen.findByRole("menuitem", { name: label })).toBeDefined()
    }
  })

  it("keeps a folder's delete for a manager", async () => {
    stubServices()
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))

    expect(await screen.findByRole("menuitem", { name: "Delete" })).toBeDefined()
    expect(screen.getByRole("menuitem", { name: "Rename" })).toBeDefined()
  })

  it("offers the share to a manager and not to an editor", async () => {
    stubServices([saved({ id: "q1", title: "Revenue", level: "manage" })])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Revenue" }))

    // Both access routes of a query ask for **manage** (C3).
    expect(await screen.findByRole("menuitem", { name: "Share" })).toBeDefined()
  })

  it("keeps the share away from an editor", async () => {
    stubServices([saved({ id: "q1", title: "Revenue", level: "edit" })])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Revenue" }))

    expect(await screen.findByRole("menuitem", { name: "Rename" })).toBeDefined()
    expect(menuItems("Share")).toBe(0)
  })

  it("opens the share panel of a query from its row menu", async () => {
    stubServices([saved({ id: "q1", title: "Revenue", level: "manage" })])
    const access = spyOn(queryService, "getSavedAccess").mockResolvedValue({
      itemType: "query",
      itemId: "q1",
      yourLevel: "manage",
      rules: [],
      inherited: [],
    })
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Revenue" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Share" }))

    await waitFor(() => {
      expect(access.mock.calls.at(-1)?.[0]).toBe("q1")
    })
    expect(await screen.findByText("Share query")).toBeDefined()
  })

  it("offers a folder's share to a manager", async () => {
    stubServices()
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))

    expect(await screen.findByRole("menuitem", { name: "Share" })).toBeDefined()
  })

  it("keeps a folder's share away from an editor", async () => {
    stubServices()
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Quarterly" }))

    expect(await screen.findByRole("menuitem", { name: "Rename" })).toBeDefined()
    expect(menuItems("Share")).toBe(0)
  })

  it("offers an editor a folder's rename but no folder's delete", async () => {
    stubServices()
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Quarterly" }))

    expect(await screen.findByRole("menuitem", { name: "Rename" })).toBeDefined()
    expect(menuItems("Delete")).toBe(0)
  })

  it("never offers the personal space a rename or a delete", async () => {
    stubServices()
    renderPage()

    fireEvent.click(
      await screen.findByRole("button", { name: "Actions for Personal space (only you)" }),
    )

    // A personal space holds folders of one's own, but it is never renamed
    // or deleted.
    expect(await screen.findByRole("menuitem", { name: "New folder" })).toBeDefined()
    expect(menuItems("Rename")).toBe(0)
    expect(menuItems("Delete")).toBe(0)
    // Nor is it shared: it is one person's own space.
    expect(menuItems("Share")).toBe(0)
  })

  it("makes a folder in the shared space with the name typed in the dialog", async () => {
    stubServices()
    const create = spyOn(queryService, "createFolder").mockResolvedValue(FOLDERS[1])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "New folder" }))
    fireEvent.change(await screen.findByLabelText("Folder name"), {
      target: { value: "Reports" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    await waitFor(() => {
      expect(create.mock.calls.at(-1)?.[0]).toEqual({ name: "Reports", parentId: null })
    })
  })

  it("renames a query with the name typed in the dialog", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "edit" })])
    const update = spyOn(queryService, "updateSaved").mockResolvedValue(
      saved({ id: "q2", title: "Costs by month", level: "edit" }),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rename" }))

    fireEvent.change(await screen.findByLabelText("Query title"), {
      target: { value: "Costs by month" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => {
      expect(update.mock.calls.at(-1)?.[0]).toBe("q2")
      expect(update.mock.calls.at(-1)?.[1]).toEqual({ title: "Costs by month" })
    })
  })

  it("takes a query to the trash", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "edit" })])
    const remove = spyOn(queryService, "deleteSaved").mockResolvedValue(undefined)
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }))

    // The query waits for the confirmation before it goes.
    expect(remove.mock.calls.length).toBe(0)
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }))

    await waitFor(() => {
      expect(remove.mock.calls.at(-1)?.[0]).toBe("q2")
    })
  })

  it("renames a folder with the name typed in the dialog", async () => {
    stubServices()
    const update = spyOn(queryService, "updateFolder").mockResolvedValue(FOLDERS[1])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rename" }))

    fireEvent.change(await screen.findByLabelText("Folder name"), {
      target: { value: "The team" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => {
      expect(update.mock.calls.at(-1)?.[0]).toBe("team")
      expect(update.mock.calls.at(-1)?.[1]).toEqual({ name: "The team" })
    })
  })

  it("deletes a folder a manager may delete", async () => {
    stubServices()
    const remove = spyOn(queryService, "deleteFolder").mockResolvedValue(undefined)
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }))
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }))

    await waitFor(() => {
      expect(remove.mock.calls.at(-1)?.[0]).toBe("team")
    })
  })

  it("shows the server's sentence when a duplicate is refused", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "view" })])
    spyOn(queryService, "duplicateSaved").mockRejectedValue(
      new ServiceError(
        "invalid_request",
        "This query holds a statement that can never run.",
      ),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Duplicate" }))

    expect(
      await screen.findByText("This query holds a statement that can never run."),
    ).toBeDefined()
  })

  it("shows the server's sentence when it refuses, and makes up none of its own", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "edit" })])
    spyOn(queryService, "deleteSaved").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "Only someone who manages this can move it into or out of a personal space.",
      ),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }))
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }))

    expect(
      await screen.findByText(
        "Only someone who manages this can move it into or out of a personal space.",
      ),
    ).toBeDefined()
  })

  it("shows the server's sentence when it refuses a folder change", async () => {
    stubServices()
    spyOn(queryService, "deleteFolder").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "You do not have permission to change this folder.",
      ),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }))
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }))

    expect(
      await screen.findByText("You do not have permission to change this folder."),
    ).toBeDefined()
  })

  it("offers the move to an editor and not to a viewer", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "edit" })])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))

    expect(await screen.findByRole("menuitem", { name: "Move" })).toBeDefined()
  })

  it("keeps the move away from a viewer, and offers the history to everybody", async () => {
    stubServices([saved({ id: "q1", title: "Revenue", level: "view" })])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Revenue" }))

    // Seeing the history is part of seeing the query (C2).
    expect(await screen.findByRole("menuitem", { name: "History" })).toBeDefined()
    expect(menuItems("Move")).toBe(0)
  })

  it("moves a query into the shared space", async () => {
    stubServices([
      saved({ id: "q2", title: "Costs", level: "edit", folderId: "team" }),
    ])
    const update = spyOn(queryService, "updateSaved").mockResolvedValue(
      saved({ id: "q2", title: "Costs", level: "edit", folderId: null }),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    fireEvent.change(await screen.findByLabelText("Target folder"), {
      target: { value: "" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    await waitFor(() => {
      expect(update.mock.calls.at(-1)?.[0]).toBe("q2")
      // `null` is the shared space: the server reads it apart from an
      // absent field.
      expect(update.mock.calls.at(-1)?.[1]).toEqual({ folderId: null })
    })
  })

  it("moves a query into a folder", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "edit" })])
    const update = spyOn(queryService, "updateSaved").mockResolvedValue(
      saved({ id: "q2", title: "Costs", level: "edit", folderId: "team" }),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    fireEvent.change(await screen.findByLabelText("Target folder"), {
      target: { value: "team" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    await waitFor(() => {
      expect(update.mock.calls.at(-1)?.[0]).toBe("q2")
      expect(update.mock.calls.at(-1)?.[1]).toEqual({ folderId: "team" })
    })
  })

  it("shows the server's sentence when a move across a personal space is refused", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "edit" })])
    spyOn(queryService, "updateSaved").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "permission_denied: Only someone who manages this can move it into or out of a personal space.",
      ),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    fireEvent.change(await screen.findByLabelText("Target folder"), {
      target: { value: "personal-1" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    // The console does not guess the rule and does not block the move
    // before the server answers: it shows the sentence the server sent (C6).
    expect((await screen.findByRole("alert")).textContent).toBe(
      "Only someone who manages this can move it into or out of a personal space.",
    )
  })

  it("forgets the last refusal when the move dialog opens again", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "edit" })])
    spyOn(queryService, "updateSaved").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "permission_denied: Only someone who manages this can move it into or out of a personal space.",
      ),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    fireEvent.change(await screen.findByLabelText("Target folder"), {
      target: { value: "personal-1" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Move" }))
    expect((await screen.findByRole("alert")).textContent).toBe(
      "Only someone who manages this can move it into or out of a personal space.",
    )

    // The person closes the dialog and comes back to it: the refusal of
    // the last attempt is gone, so it is not read as the answer to this
    // one.
    fireEvent.click(screen.getByRole("button", { name: "Close" }))
    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))

    expect(screen.queryAllByRole("alert").length).toBe(0)
  })

  it("moves a folder under another folder", async () => {
    stubServices()
    const update = spyOn(queryService, "updateFolder").mockResolvedValue(FOLDERS[1])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    fireEvent.change(await screen.findByLabelText("Target folder"), {
      target: { value: "quarterly" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    await waitFor(() => {
      expect(update.mock.calls.at(-1)?.[0]).toBe("team")
      expect(update.mock.calls.at(-1)?.[1]).toEqual({ parentId: "quarterly" })
    })
  })

  it("moves a folder into the shared space", async () => {
    stubServices()
    const update = spyOn(queryService, "updateFolder").mockResolvedValue(FOLDERS[1])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    fireEvent.change(await screen.findByLabelText("Target folder"), {
      target: { value: "" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    await waitFor(() => {
      expect(update.mock.calls.at(-1)?.[0]).toBe("team")
      expect(update.mock.calls.at(-1)?.[1]).toEqual({ parentId: null })
    })
  })

  it("never offers the personal space a move", async () => {
    stubServices()
    renderPage()

    fireEvent.click(
      await screen.findByRole("button", { name: "Actions for Personal space (only you)" }),
    )

    // A personal space is one person's own space: it is never moved.
    expect(await screen.findByRole("menuitem", { name: "New folder" })).toBeDefined()
    expect(menuItems("Move")).toBe(0)
  })

  it("opens the trash from the list", async () => {
    stubServices()
    spyOn(queryService, "listSavedTrash").mockResolvedValue([])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Trash" }))

    expect(await screen.findByText("The trash is empty.")).toBeDefined()
  })
})
