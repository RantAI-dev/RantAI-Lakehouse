// The share panel of a saved query and of a folder (`QS-3` C3): the people
// and the roles that may open the item, the levels it inherits from the
// folders above it, and the picker that adds a grantee.
//
// The server replaces the WHOLE rule list in one write
// (`PutAccessBody {rules}`), so the panel sends the complete list every
// time it saves: what is there, less what was removed, plus what was
// added. A rule names one person or one role and one level, and the body
// carries nothing else — the server refuses a body of another shape.
//
// The service is spied; nothing leaves the test.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { queryService } from "@/services"
import type { GranteeList, SavedQueryAccessList } from "@/services/contracts/queries"
import { ServiceError } from "@/services/errors"
import { SavedQueryAccessPanel } from "./saved-query-access-panel"

afterEach(() => {
  cleanup()
  mock.restore()
})

/**
 * The access list the server sends for a query: two rules of its own (one
 * person, one role) and one rule it inherits from the folder above it.
 */
const ACCESS: SavedQueryAccessList = {
  itemType: "query",
  itemId: "q1",
  yourLevel: "manage",
  rules: [
    {
      id: "rule-a",
      userId: "u-2",
      roleId: null,
      granteeName: "Dewi Lestari",
      granteeEmail: "dewi@example.com",
      level: "edit",
      grantedBy: "u-1",
      grantedAt: "2026-11-01T09:00:00.000Z",
    },
    {
      id: "rule-b",
      userId: null,
      roleId: "role-1",
      granteeName: "Finance",
      granteeEmail: null,
      level: "view",
      grantedBy: null,
      grantedAt: "2026-11-02T09:00:00.000Z",
    },
  ],
  inherited: [
    {
      folderId: "team",
      folderName: "Team",
      userId: "u-3",
      roleId: null,
      granteeName: "Bagus Santoso",
      granteeEmail: "bagus@example.com",
      level: "view",
    },
  ],
}

/** The people and the roles the server offers when the search finds them. */
const GRANTEES: GranteeList = {
  people: [{ id: "u-9", name: "Reza Hakim", email: "reza@example.com" }],
  roles: [{ id: "role-9", name: "Analyst" }],
}

function stubAccess(over: Partial<SavedQueryAccessList> = {}) {
  return spyOn(queryService, "getSavedAccess").mockResolvedValue({ ...ACCESS, ...over })
}

function stubFolderAccess() {
  return spyOn(queryService, "getFolderAccess").mockResolvedValue({
    ...ACCESS,
    itemType: "folder",
    itemId: "team",
  })
}

function stubGrantees() {
  return spyOn(queryService, "findGrantees").mockResolvedValue(GRANTEES)
}

function renderPanel(kind: "query" | "folder" = "query") {
  return render(
    <SavedQueryAccessPanel
      kind={kind}
      id={kind === "query" ? "q1" : "team"}
      title={kind === "query" ? "Revenue" : "Team"}
      onClose={() => {}}
    />,
  )
}

describe("the share panel", () => {
  it("lists the people and the roles with their level", async () => {
    stubAccess()
    renderPanel()

    await screen.findByText("Dewi Lestari")

    const own = screen.getByRole("list", { name: "People and roles with access" })
    expect(own.textContent).toContain("Dewi Lestari")
    expect(own.textContent).toContain("Finance")
    expect(own.textContent).toContain("Can edit")
    expect(own.textContent).toContain("Can view")
  })

  it("keeps the levels inherited from the folders above in a group of their own", async () => {
    stubAccess()
    renderPanel()

    await screen.findByText("Dewi Lestari")

    const inherited = screen.getByRole("list", { name: "Inherited access" })
    expect(inherited.textContent).toContain("Bagus Santoso")
    expect(inherited.textContent).toContain("Team")
    // An inherited rule is not one of the item's own rules.
    const own = screen.getByRole("list", { name: "People and roles with access" })
    expect(own.textContent).not.toContain("Bagus Santoso")
  })

  it("names a person with the email under it and a role with (role) after it", async () => {
    stubAccess()
    renderPanel()

    const person = (await screen.findByText("Dewi Lestari")).closest("li")
    expect(person?.textContent).toContain("dewi@example.com")
    // The id is the server's business; the panel shows who the rule names.
    expect(person?.textContent).not.toContain("u-2")

    const role = screen.getByText("Finance").closest("li")
    expect(role?.textContent).toContain("Finance")
    expect(role?.textContent).toContain("(role)")
    expect(role?.textContent).not.toContain("role-1")
  })

  it("labels the level picker and the remove button with the grantee's name", async () => {
    stubAccess()
    renderPanel()

    await screen.findByText("Dewi Lestari")

    expect(screen.getByLabelText("Level of Dewi Lestari")).toBeDefined()
    expect(screen.getByRole("button", { name: "Remove Dewi Lestari" })).toBeDefined()
    expect(screen.getByRole("button", { name: "Remove Finance (role)" })).toBeDefined()
  })

  it("says Unknown person or Unknown role, with the id, when the server found no name", async () => {
    stubAccess({
      rules: [
        {
          id: "rule-x",
          userId: "u-77",
          roleId: null,
          granteeName: null,
          granteeEmail: null,
          level: "view",
          grantedBy: null,
          grantedAt: "2026-11-02T09:00:00.000Z",
        },
        {
          id: "rule-y",
          userId: null,
          roleId: "role-77",
          granteeName: null,
          granteeEmail: null,
          level: "view",
          grantedBy: null,
          grantedAt: "2026-11-02T09:00:00.000Z",
        },
      ],
      inherited: [],
    })
    renderPanel()

    await screen.findByText("Unknown person")
    const own = screen.getByRole("list", { name: "People and roles with access" })
    expect(own.textContent).toContain("Unknown person")
    expect(own.textContent).toContain("u-77")
    expect(own.textContent).toContain("Unknown role")
    expect(own.textContent).toContain("role-77")
    // The two unknowns stay apart for a screen reader.
    expect(screen.getByRole("button", { name: "Remove Unknown person (u-77)" })).toBeDefined()
    expect(screen.getByRole("button", { name: "Remove Unknown role (role-77)" })).toBeDefined()
  })

  it("keeps the name of a grantee picked from the search until the list is saved", async () => {
    stubAccess()
    stubGrantees()
    renderPanel()

    fireEvent.change(await screen.findByLabelText("Add a person or role"), {
      target: { value: "re" },
    })
    fireEvent.click(await screen.findByRole("button", { name: /Reza Hakim/ }))

    const own = screen.getByRole("list", { name: "People and roles with access" })
    expect(own.textContent).toContain("Reza Hakim")
    expect(own.textContent).toContain("reza@example.com")
    expect(own.textContent).not.toContain("u-9")
  })

  it("says in one line what the shared space means", async () => {
    stubAccess()
    renderPanel()

    expect(
      await screen.findByText(
        "In the shared space everyone in the organisation can already view.",
      ),
    ).toBeDefined()
  })

  it("asks the server for the people and the roles that match the text typed", async () => {
    stubAccess()
    const find = stubGrantees()
    renderPanel()

    fireEvent.change(await screen.findByLabelText("Add a person or role"), {
      target: { value: "reza" },
    })

    await waitFor(() => {
      expect(find.mock.calls.at(-1)?.[0]).toBe("reza")
    })
  })

  it("shows the shared fallback when the grantee search fails with something that is not an error", async () => {
    stubAccess()
    spyOn(queryService, "findGrantees").mockRejectedValue("not an error")
    renderPanel()

    fireEvent.change(await screen.findByLabelText("Add a person or role"), {
      target: { value: "reza" },
    })

    // A refusal is the server's sentence (C6): a thrown value that is not
    // an error has no sentence of its own, so the console shows the shared
    // fallback and makes up none of its own.
    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe("Unexpected error.")
    expect(screen.queryAllByText("The search failed.").length).toBe(0)
  })

  it("writes the whole list when a person and a role are added", async () => {
    stubAccess()
    stubGrantees()
    const put = spyOn(queryService, "putSavedAccess").mockResolvedValue(ACCESS)
    renderPanel()

    fireEvent.change(await screen.findByLabelText("Add a person or role"), {
      target: { value: "re" },
    })
    chooseOption(screen.getByRole("combobox", { name: "Level to give" }), "Can edit")
    fireEvent.click(await screen.findByRole("button", { name: /Reza Hakim/ }))
    fireEvent.change(screen.getByLabelText("Add a person or role"), {
      target: { value: "ana" },
    })
    chooseOption(screen.getByRole("combobox", { name: "Level to give" }), "Can view")
    fireEvent.click(await screen.findByRole("button", { name: /Analyst/ }))
    fireEvent.click(screen.getByRole("button", { name: "Save access" }))

    await waitFor(() => {
      expect(put.mock.calls.length).toBe(1)
    })
    const body = put.mock.calls.at(-1)?.[1]
    expect(body).toEqual({
      rules: [
        { userId: "u-2", level: "edit" },
        { roleId: "role-1", level: "view" },
        { userId: "u-9", level: "edit" },
        { roleId: "role-9", level: "view" },
      ],
    })
    // A rule names one grantee and one level, and nothing else: the server
    // refuses a body of another shape.
    const rules = (body as { rules: Record<string, unknown>[] }).rules
    for (const rule of rules) {
      expect(Object.keys(rule).sort()).toEqual(
        rule.userId ? ["level", "userId"] : ["level", "roleId"],
      )
    }
  })

  it("writes the whole list when a rule is removed and another rule's level changes", async () => {
    stubAccess()
    const put = spyOn(queryService, "putSavedAccess").mockResolvedValue(ACCESS)
    renderPanel()

    await screen.findByText("Dewi Lestari")

    fireEvent.click(screen.getByRole("button", { name: "Remove Dewi Lestari" }))
    chooseOption(screen.getByRole("combobox", { name: "Level of Finance (role)" }), "Can manage")
    fireEvent.click(screen.getByRole("button", { name: "Save access" }))

    await waitFor(() => {
      expect(put.mock.calls.length).toBe(1)
    })
    // The removal and the new level go in one write: the server replaces
    // the whole list, so the panel sends what it wants the list to be.
    expect(put.mock.calls.at(-1)?.[1]).toEqual({
      rules: [{ roleId: "role-1", level: "manage" }],
    })
  })

  it("shows the list the server sent after the write", async () => {
    stubAccess()
    spyOn(queryService, "putSavedAccess").mockResolvedValue({
      ...ACCESS,
      rules: [
        {
          id: "rule-c",
          userId: "u-9",
          roleId: null,
          granteeName: "Reza Hakim",
          granteeEmail: "reza@example.com",
          level: "manage",
          grantedBy: "u-1",
          grantedAt: "2026-11-03T09:00:00.000Z",
        },
      ],
    })
    renderPanel()

    await screen.findByText("Dewi Lestari")

    fireEvent.click(screen.getByRole("button", { name: "Remove Dewi Lestari" }))
    fireEvent.click(screen.getByRole("button", { name: "Save access" }))

    // The panel draws the answer of the write, not the list it held.
    expect(await screen.findByText("Reza Hakim")).toBeDefined()
    expect(screen.queryAllByText("Dewi Lestari").length).toBe(0)
  })

  it("shows the server's sentence when the write is refused", async () => {
    stubAccess()
    spyOn(queryService, "putSavedAccess").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "permission_denied: You do not have permission to change this saved query.",
      ),
    )
    renderPanel()

    await screen.findByText("Dewi Lestari")

    fireEvent.click(screen.getByRole("button", { name: "Remove Dewi Lestari" }))
    fireEvent.click(screen.getByRole("button", { name: "Save access" }))

    expect((await screen.findByRole("alert")).textContent).toBe(
      "You do not have permission to change this saved query.",
    )
  })

  it("shows a folder's refusal inside the share dialog, in the server's own sentence", async () => {
    stubFolderAccess()
    spyOn(queryService, "putFolderAccess").mockRejectedValue(
      new ServiceError(
        "permission_denied",
        "permission_denied: Only someone who manages everything in this folder can share it.",
      ),
    )
    renderPanel("folder")

    await screen.findByText("Dewi Lestari")

    fireEvent.click(screen.getByRole("button", { name: "Remove Dewi Lestari" }))
    fireEvent.click(screen.getByRole("button", { name: "Save access" }))

    const dialog = await screen.findByRole("dialog")
    expect((await within(dialog).findByRole("alert")).textContent).toBe(
      "Only someone who manages everything in this folder can share it.",
    )
  })

  it("shows the server's sentence when the body is refused", async () => {
    stubAccess()
    spyOn(queryService, "putSavedAccess").mockRejectedValue(
      new ServiceError(
        "invalid_request",
        "invalid_request: Body must be JSON {rules: [{userId?, roleId?, level}]}.",
      ),
    )
    renderPanel()

    await screen.findByText("Dewi Lestari")

    fireEvent.click(screen.getByRole("button", { name: "Remove Dewi Lestari" }))
    fireEvent.click(screen.getByRole("button", { name: "Save access" }))

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Body must be JSON {rules: [{userId?, roleId?, level}]}.",
    )
  })

  it("shows the server's sentence when the list cannot be read", async () => {
    spyOn(queryService, "getSavedAccess").mockRejectedValue(
      new ServiceError("not_found", "not_found: That saved query could not be found."),
    )
    renderPanel()

    expect((await screen.findByRole("alert")).textContent).toBe(
      "That saved query could not be found.",
    )
  })

  it("reads and writes a folder through the folder's own routes", async () => {
    const get = spyOn(queryService, "getFolderAccess").mockResolvedValue({
      ...ACCESS,
      itemType: "folder",
      itemId: "team",
    })
    const put = spyOn(queryService, "putFolderAccess").mockResolvedValue({
      ...ACCESS,
      itemType: "folder",
      itemId: "team",
    })
    renderPanel("folder")

    await screen.findByText("Dewi Lestari")
    expect(get.mock.calls.at(-1)?.[0]).toBe("team")

    fireEvent.click(screen.getByRole("button", { name: "Remove Dewi Lestari" }))
    fireEvent.click(screen.getByRole("button", { name: "Save access" }))

    await waitFor(() => {
      expect(put.mock.calls.length).toBe(1)
    })
    expect(put.mock.calls.at(-1)?.[0]).toBe("team")
    expect(put.mock.calls.at(-1)?.[1]).toEqual({
      rules: [{ roleId: "role-1", level: "view" }],
    })
  })
})
