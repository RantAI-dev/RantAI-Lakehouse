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

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { AuthProvider } from "@/features/auth/auth-provider"
import { queryService } from "@/services"
import type {
  QueryFolder,
  SavedQuery,
  SavedQueryAccessList,
} from "@/services/contracts/queries"
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
    parameters: [],
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

/** A folder's access list with one rule for the caller (`u1`) and one for Dewi. */
function sharedFolderAccess(): SavedQueryAccessList {
  const rule = (userId: string, granteeName: string) => ({
    id: `rule-${userId}`,
    userId,
    roleId: null,
    granteeName,
    granteeEmail: null,
    level: "edit",
    grantedBy: null,
    grantedAt: "2026-11-01T09:00:00.000Z",
  })
  return {
    itemType: "folder",
    itemId: "quarterly",
    yourLevel: "edit",
    rules: [rule("u1", "Rina Wijaya"), rule("u2", "Dewi Lestari")],
    inherited: [],
  }
}

function renderPage() {
  // The move dialogs read the access list of the folder they are aimed at;
  // a test that cares about it spies the call again.
  spyOn(queryService, "getFolderAccess").mockResolvedValue({
    ...sharedFolderAccess(),
    rules: [],
  })
  // The page reads the signed-in person from `AuthProvider`, as the root
  // layout supplies it; the session read is answered here, not by a server.
  spyOn(globalThis, "fetch").mockImplementation((async () =>
    new Response(
      JSON.stringify({
        id: "u1",
        name: "Rina Wijaya",
        email: null,
        roles: [],
        permissions: [],
        tenants: [],
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    )) as unknown as typeof fetch)
  return render(
    <AuthProvider>
      <TableProviders>
        <SavedQueriesPage />
      </TableProviders>
    </AuthProvider>,
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

  it("offers an editor the rename and the delete, and keeps the move away", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "edit" })])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))

    expect(await screen.findByRole("menuitem", { name: "Rename" })).toBeDefined()
    expect(screen.getByRole("menuitem", { name: "Delete" })).toBeDefined()
    // A move needs **manage**: an editor changes the query, not where it
    // is seen from (`QS-3` review of #382).
    expect(menuItems("Move")).toBe(0)
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
      "Move",
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
        "Only someone who manages this can move it.",
      ),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }))
    const dialog = await screen.findByRole("dialog")
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }))

    // The dialog is where the person is looking: the sentence is said
    // there, not somewhere behind it.
    expect(
      (await within(dialog).findByRole("alert")).textContent,
    ).toBe("Only someone who manages this can move it.")
  })

  it("forgets the last query-delete refusal when the confirm dialog opens again", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "edit" })])
    spyOn(queryService, "deleteSaved").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "Only someone who manages this can move it.",
      ),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }))
    const first = await screen.findByRole("dialog")
    fireEvent.click(within(first).getByRole("button", { name: "Delete" }))
    await within(first).findByRole("alert")

    fireEvent.click(within(first).getByRole("button", { name: "Cancel" }))
    await waitFor(() => {
      expect(screen.queryAllByRole("dialog").length).toBe(0)
    })
    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }))
    const second = await screen.findByRole("dialog")

    // The refusal belonged to the last attempt, not to this one.
    expect(within(second).queryAllByRole("alert").length).toBe(0)
  })

  it("shows the refusal of a folder delete inside its confirm dialog", async () => {
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
    const dialog = await screen.findByRole("dialog")
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }))

    // The dialog is where the person is looking: the sentence is said
    // there, not below the tree behind it (#389 SHOULD-FIX 1).
    expect(
      (await within(dialog).findByRole("alert")).textContent,
    ).toBe("You do not have permission to change this folder.")
  })

  it("forgets the last folder-delete refusal when the confirm dialog opens again", async () => {
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
    const first = await screen.findByRole("dialog")
    fireEvent.click(within(first).getByRole("button", { name: "Delete" }))
    await within(first).findByRole("alert")

    fireEvent.click(within(first).getByRole("button", { name: "Cancel" }))
    await waitFor(() => {
      expect(screen.queryAllByRole("dialog").length).toBe(0)
    })
    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }))
    const second = await screen.findByRole("dialog")

    // The refusal belonged to the last attempt, not to this one.
    expect(within(second).queryAllByRole("alert").length).toBe(0)
  })

  it("offers the move of a query to a manager", async () => {
    stubServices([saved({ id: "q3", title: "Forecast", level: "manage" })])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Forecast" }))

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
      saved({ id: "q2", title: "Costs", level: "manage", folderId: "team" }),
    ])
    const update = spyOn(queryService, "updateSaved").mockResolvedValue(
      saved({ id: "q2", title: "Costs", level: "manage", folderId: null }),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    chooseOption(await screen.findByRole("combobox", { name: "Target folder" }), "Shared")
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    await waitFor(() => {
      expect(update.mock.calls.at(-1)?.[0]).toBe("q2")
      // `null` is the shared space: the server reads it apart from an
      // absent field.
      expect(update.mock.calls.at(-1)?.[1]).toEqual({ folderId: null })
    })
  })

  it("moves a query into a folder", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "manage" })])
    const update = spyOn(queryService, "updateSaved").mockResolvedValue(
      saved({ id: "q2", title: "Costs", level: "manage", folderId: "team" }),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    chooseOption(await screen.findByRole("combobox", { name: "Target folder" }), "Team")
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    await waitFor(() => {
      expect(update.mock.calls.at(-1)?.[0]).toBe("q2")
      expect(update.mock.calls.at(-1)?.[1]).toEqual({ folderId: "team" })
    })
  })

  it("shows the server's sentence when a move is refused", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "manage" })])
    spyOn(queryService, "updateSaved").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "permission_denied: Only someone who manages this can move it.",
      ),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    chooseOption(
      await screen.findByRole("combobox", { name: "Target folder" }),
      "Personal space (only you)",
    )
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    // The console does not guess the rule and does not block the move
    // before the server answers: it shows the sentence the server sent (C6).
    expect((await screen.findByRole("alert")).textContent).toBe(
      "Only someone who manages this can move it.",
    )
  })

  it("forgets the last refusal when the move dialog opens again", async () => {
    stubServices([saved({ id: "q2", title: "Costs", level: "manage" })])
    spyOn(queryService, "updateSaved").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "permission_denied: Only someone who manages this can move it.",
      ),
    )
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Costs" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    chooseOption(
      await screen.findByRole("combobox", { name: "Target folder" }),
      "Personal space (only you)",
    )
    fireEvent.click(screen.getByRole("button", { name: "Move" }))
    expect((await screen.findByRole("alert")).textContent).toBe(
      "Only someone who manages this can move it.",
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
    chooseOption(
      await screen.findByRole("combobox", { name: "Target folder" }),
      "Team / Quarterly",
    )
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    await waitFor(() => {
      expect(update.mock.calls.at(-1)?.[0]).toBe("team")
      expect(update.mock.calls.at(-1)?.[1]).toEqual({ parentId: "quarterly" })
    })
  })

  it("tells the person moving a query who the target folder is shared with, without themselves", async () => {
    stubServices([saved({ id: "q1", title: "Revenue", level: "manage" })])
    renderPage()
    spyOn(queryService, "getFolderAccess").mockResolvedValue(sharedFolderAccess())

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Revenue" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    chooseOption(
      await screen.findByRole("combobox", { name: "Target folder" }),
      "Team / Quarterly",
    )

    // `u1` is the signed-in person (the session read in `renderPage`).
    expect(await screen.findByText("Shared with 1 person or role: Dewi Lestari.")).toBeDefined()
  })

  it("tells the person moving a folder who the target folder is shared with, without themselves", async () => {
    stubServices()
    renderPage()
    spyOn(queryService, "getFolderAccess").mockResolvedValue(sharedFolderAccess())

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    chooseOption(
      await screen.findByRole("combobox", { name: "Target folder" }),
      "Team / Quarterly",
    )

    expect(await screen.findByText("Shared with 1 person or role: Dewi Lestari.")).toBeDefined()
  })

  it("moves a folder into the shared space", async () => {
    stubServices()
    const update = spyOn(queryService, "updateFolder").mockResolvedValue(FOLDERS[1])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move" }))
    chooseOption(await screen.findByRole("combobox", { name: "Target folder" }), "Shared")
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    await waitFor(() => {
      expect(update.mock.calls.at(-1)?.[0]).toBe("team")
      expect(update.mock.calls.at(-1)?.[1]).toEqual({ parentId: null })
    })
  })

  it("offers a folder's move to a manager and its rename to an editor", async () => {
    stubServices()
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Team" }))
    expect(await screen.findByRole("menuitem", { name: "Move" })).toBeDefined()
    expect(screen.getByRole("menuitem", { name: "Rename" })).toBeDefined()
  })

  it("keeps a folder's move away from an editor, who may still rename it", async () => {
    stubServices()
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Actions for Quarterly" }))

    // A folder move changes who sees everything inside it: it needs
    // **manage**, where a rename needs **edit** (`QS-3` review of #382).
    expect(await screen.findByRole("menuitem", { name: "Rename" })).toBeDefined()
    expect(menuItems("Move")).toBe(0)
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

  it("says no saved query matches when a search finds none, and not that none exist", async () => {
    spyOn(queryService, "listSavedFiltered").mockImplementation(
      (async (filter?: { q?: string }) =>
        filter?.q ? [] : [saved({ id: "q1", title: "Zebra" })]) as typeof queryService.listSavedFiltered,
    )
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    spyOn(queryService, "getSchedulingCapability").mockResolvedValue({
      supported: false,
      reason: "not yet",
    })
    renderPage()
    await screen.findByText("Zebra")

    fireEvent.change(screen.getByLabelText("Search saved queries"), {
      target: { value: "nothing like this" },
    })

    expect(await screen.findByText("No saved queries match.", {}, { timeout: 2000 })).toBeDefined()
    // The Studio hint is for a caller who has saved nothing.
    expect(screen.queryAllByText(/Write a query in the Studio/).length).toBe(0)
  })

  it("says no saved query matches when the folder selected holds none", async () => {
    const { list } = stubServices([])
    list.mockImplementation(
      (async (filter?: { folder?: string }) =>
        filter?.folder ? [] : [saved({ id: "q1", title: "Zebra" })]) as typeof queryService.listSavedFiltered,
    )
    renderPage()
    await screen.findByText("Zebra")

    fireEvent.click(screen.getByRole("button", { name: "Quarterly" }))

    expect(await screen.findByText("No saved queries match.")).toBeDefined()
    expect(screen.queryAllByText(/Write a query in the Studio/).length).toBe(0)
  })

  it("keeps the Studio hint for a caller who has saved nothing", async () => {
    stubServices([])
    renderPage()

    expect(await screen.findByText("No saved queries")).toBeDefined()
    expect(screen.getByText(/Write a query in the Studio/)).toBeDefined()
    expect(screen.queryAllByText("No saved queries match.").length).toBe(0)
  })

  it("opens the trash from the list", async () => {
    stubServices()
    spyOn(queryService, "listSavedTrash").mockResolvedValue([])
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Trash" }))

    expect(await screen.findByText("The trash is empty.")).toBeDefined()
  })
})
