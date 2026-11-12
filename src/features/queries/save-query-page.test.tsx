// The Save query page of Query Studio (`QS-3` C4, `WEB-7` T5/D5): where a
// query lands, and what the page says when it is opened.
//
// The page reads the statement the editor left for it in `sessionStorage`
// (`save-draft.ts`); the folders and the query the tab came from are stubbed
// here, so nothing leaves the test.

let pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/query-studio/save",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: (href: string) => {
      pushed.push(href)
    },
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

// The page asks who is signed in only to leave them out of "Shared with";
// the provider would fetch the session, so the hook alone is replaced.
let viewerId: string | undefined = "u1"
import * as realAuth from "@/features/auth/auth-provider"
const real = { ...realAuth }
mock.module("@/features/auth/auth-provider", () => ({
  ...real,
  useAuth: () => ({
    user: viewerId ? { id: viewerId } : null,
    hasPermission: () => true,
    signOut: () => {},
    refresh: () => {},
    loading: false,
  }),
}))

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { readTabs, writeTabs } from "@/lib/query-draft"
import { queryService } from "@/services"
import type {
  QueryFolder,
  QueryParamDef,
  SaveQueryInput,
  SavedQuery,
  SavedQueryAccessList,
} from "@/services/contracts/queries"
import { ServiceError } from "@/services/errors"
import { SaveQueryPage } from "./save-query-page"
import { readSaveQueryDraft, writeSaveQueryDraft } from "./save-draft"

afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
})

afterEach(() => {
  cleanup()
  mock.restore()
  window.sessionStorage.clear()
  window.localStorage.clear()
  viewerId = "u1"
  pushed = []
})

beforeEach(() => {
  pushed = []
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
  parameters: [],
}

/** The caller of these tests: the person who saves, never named in the line. */
const VIEWER = "u1"

/**
 * The access list of a folder, as the server sends it: its own rules, then
 * the rules it inherits. A rule names one grantee; the caller's own rule
 * (`u1`) is in it, as it is on a real folder the caller authored.
 */
function accessList(
  rules: SavedQueryAccessList["rules"],
  inherited: SavedQueryAccessList["inherited"] = []
): SavedQueryAccessList {
  return { itemType: "folder", itemId: "team", yourLevel: "edit", rules, inherited }
}

function rule(
  userId: string | null,
  roleId: string | null,
  granteeName: string | null
): SavedQueryAccessList["rules"][number] {
  return {
    id: `rule-${userId ?? roleId}`,
    userId,
    roleId,
    granteeName,
    granteeEmail: null,
    level: "edit",
    grantedBy: null,
    grantedAt: "2026-11-01T09:00:00.000Z",
  }
}

function inheritedRule(
  userId: string | null,
  roleId: string | null,
  granteeName: string | null
): SavedQueryAccessList["inherited"][number] {
  return {
    folderId: "root-team",
    folderName: "Company",
    userId,
    roleId,
    granteeName,
    granteeEmail: null,
    level: "view",
  }
}

/** A folder that nobody but the caller and Dewi, Finance can reach. */
const TEAM_ACCESS = accessList(
  [rule("u1", null, "Rina Wijaya"), rule("u2", null, "Dewi Lestari"), rule(null, "r1", "Finance")],
  // Dewi again, through the folder above: one grantee, counted once.
  [inheritedRule("u2", null, "Dewi Lestari")]
)

/**
 * The draft the editor would have left for this page. The tab id is the one
 * the editor's own draft holds, so a save can hand the new id back to it.
 */
function seedDraft(
  over: {
    readonly mode?: "create" | "update"
    readonly savedId?: string | null
    readonly sql?: string
    readonly parameters?: QueryParamDef[]
  } = {}
): void {
  writeSaveQueryDraft({
    sql: over.sql ?? "SELECT 1",
    parameters: over.parameters ?? [],
    mode: over.mode ?? "create",
    savedId: over.savedId ?? null,
    tabId: "tab-1",
  })
}

/**
 * Render the page with a draft from the editor, and record the two writes.
 * `saveQuery` answers a new query (`q2`); `updateSaved` answers the query the
 * tab came from.
 */
function renderPage(
  over: {
    readonly mode?: "create" | "update"
    readonly savedId?: string | null
    readonly sql?: string
    readonly viewerId?: string
    readonly noViewer?: boolean
    readonly parameters?: QueryParamDef[]
  } = {}
) {
  viewerId = over.noViewer ? undefined : (over.viewerId ?? VIEWER)
  seedDraft(over)
  const saveQuery = spyOn(queryService, "saveQuery").mockResolvedValue({ ...SAVED, id: "q2" })
  const updateSaved = spyOn(queryService, "updateSaved").mockResolvedValue(SAVED)
  render(<SaveQueryPage />)
  return { saveQuery, updateSaved }
}

/** The folder the picker shows. */
async function picker() {
  return await screen.findByRole("combobox", { name: "Folder" })
}

/**
 * The folder picker once the folder list has been read and rendered: the
 * combobox exists before `/folders` answers, and the options do not.
 */
async function foldersReady() {
  const select = await picker()
  await act(async () => {})
  return select
}

describe("the save query page has no draft", () => {
  it("says to open it from Query Studio and links there", async () => {
    spyOn(queryService, "saveQuery").mockResolvedValue(SAVED)
    render(<SaveQueryPage />)

    expect(await screen.findByText("Open this from Query Studio")).toBeDefined()
    // The repo's Button renders a link with `role="button"`, so the href is
    // read off the anchor itself.
    expect(screen.getByText("Open Query Studio").closest("a")?.getAttribute("href")).toBe(
      "/query-studio"
    )
  })

  it("deletes the draft after a successful save and goes back to the editor", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    const { saveQuery } = renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Save as new" }))

    await waitFor(() => expect(saveQuery).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(pushed).toEqual(["/query-studio"]))
    expect(readSaveQueryDraft()).toBeNull()
  })

  it("keeps the draft when the save is refused, so nothing typed is lost", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    renderPage()
    spyOn(queryService, "saveQuery").mockRejectedValue(
      new ServiceError("invalid_request", "invalid_request: A query with that name exists.")
    )

    fireEvent.click(await screen.findByRole("button", { name: "Save as new" }))

    expect(await screen.findByText("invalid_request: A query with that name exists.")).toBeDefined()
    expect(readSaveQueryDraft()?.sql).toBe("SELECT 1")
    expect(pushed).toEqual([])
  })

  it("points the editor's tab at the query the save made, so it says Save next time", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    writeTabs({
      tabs: [
        {
          id: "tab-1",
          name: "Query 1",
          sql: "SELECT 1",
          engine: "clickhouse",
          parameters: [],
          values: {},
          savedId: "q1",
          level: "view",
        },
      ],
      activeId: "tab-1",
      question: "",
      tab: "sql",
    })
    // A copy of a query the caller may only view: the tab named `q1` before.
    const { saveQuery } = renderPage({ savedId: "q1" })
    spyOn(queryService, "getSaved").mockResolvedValue(SAVED)
    saveQuery.mockResolvedValue({ ...SAVED, id: "q2", level: "edit" })

    fireEvent.click(await screen.findByRole("button", { name: "Save as new" }))

    await waitFor(() => expect(pushed).toEqual(["/query-studio"]))
    const tab = readTabs()?.tabs[0]
    expect(tab?.savedId).toBe("q2")
    expect(tab?.level).toBe("edit")
    // The statement itself is untouched: the editor keeps its text.
    expect(tab?.sql).toBe("SELECT 1")
  })
})

describe("the save query page parameters (QS-5)", () => {
  const DEFINITIONS: QueryParamDef[] = [
    { name: "region", type: "text", default: "EU" },
    { name: "n", type: "number" },
  ]

  it("sends the definitions it is given with a new query", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    const { saveQuery } = renderPage({
      sql: "SELECT {{region}}, {{n}}",
      parameters: DEFINITIONS,
    })

    fireEvent.click(await screen.findByRole("button", { name: "Save as new" }))

    await waitFor(() => expect(saveQuery).toHaveBeenCalledTimes(1))
    const input = saveQuery.mock.calls[0]?.[0] as SaveQueryInput
    expect(input.parameters).toStrictEqual(DEFINITIONS)
  })

  it("sends no parameters key for a new query without a placeholder", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    const { saveQuery } = renderPage({ parameters: [] })

    fireEvent.click(await screen.findByRole("button", { name: "Save as new" }))

    await waitFor(() => expect(saveQuery).toHaveBeenCalledTimes(1))
    expect("parameters" in (saveQuery.mock.calls[0]?.[0] as SaveQueryInput)).toBe(false)
  })

  it("sends an empty list when a change drops the placeholders the saved query had", async () => {
    spyOn(queryService, "getSaved").mockResolvedValue({
      ...SAVED,
      level: "edit",
      parameters: DEFINITIONS,
    })
    const { updateSaved } = renderPage({ mode: "update", savedId: "q1", parameters: [] })
    await waitFor(() =>
      expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("Orders")
    )

    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(updateSaved).toHaveBeenCalledTimes(1))
    expect(updateSaved.mock.calls[0]?.[1]?.parameters).toStrictEqual([])
  })

  it("sends an empty list when a change has no placeholder, whatever the saved query held", async () => {
    spyOn(queryService, "getSaved").mockResolvedValue({
      ...SAVED,
      level: "edit",
      parameters: [],
    })
    const { updateSaved } = renderPage({ mode: "update", savedId: "q1", parameters: [] })
    await waitFor(() =>
      expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("Orders")
    )

    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(updateSaved).toHaveBeenCalledTimes(1))
    // The change says which definitions it wants, and it wants none. The
    // saved query's own definitions do not decide it.
    expect(updateSaved.mock.calls[0]?.[1]?.parameters).toStrictEqual([])
  })

  it("sends an empty list when a change has no placeholder and the saved query is not read yet", async () => {
    // The saved query is still loading: the change does not wait for it to
    // say what the definitions are.
    spyOn(queryService, "getSaved").mockReturnValue(new Promise<SavedQuery>(() => {}))
    const { updateSaved } = renderPage({ mode: "update", savedId: "q1", parameters: [] })

    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(updateSaved).toHaveBeenCalledTimes(1))
    expect(updateSaved.mock.calls[0]?.[1]?.parameters).toStrictEqual([])
  })

  it("sends an empty list when a change has no placeholder and the saved query could not be read", async () => {
    spyOn(queryService, "getSaved").mockRejectedValue(
      new ServiceError("unavailable", "unavailable: The saved query could not be read.")
    )
    const { updateSaved } = renderPage({ mode: "update", savedId: "q1", parameters: [] })

    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(updateSaved).toHaveBeenCalledTimes(1))
    expect(updateSaved.mock.calls[0]?.[1]?.parameters).toStrictEqual([])
  })

  it("sends the new definitions when a change keeps placeholders", async () => {
    spyOn(queryService, "getSaved").mockResolvedValue({
      ...SAVED,
      level: "edit",
      parameters: [],
    })
    const { updateSaved } = renderPage({
      mode: "update",
      savedId: "q1",
      sql: "SELECT {{region}}",
      parameters: [DEFINITIONS[0]!],
    })
    await waitFor(() =>
      expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("Orders")
    )

    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(updateSaved).toHaveBeenCalledTimes(1))
    expect(updateSaved.mock.calls[0]?.[1]?.parameters).toStrictEqual([DEFINITIONS[0]])
  })
})

describe("the save query page", () => {
  it("lists the folders and starts in the caller's own space", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    renderPage()

    const select = await picker()
    expect(select.textContent).toContain("Personal space (only you)")
    fireEvent.click(select)
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual([
      "Personal space (only you)",
      "Personal space (only you) / Reports",
      "Shared",
      "Team",
    ])
  })

  it("sends no folder at all for the caller's own space", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    const { saveQuery } = renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Save as new" }))

    await waitFor(() => expect(saveQuery).toHaveBeenCalledTimes(1))
    const input = saveQuery.mock.calls[0]?.[0] as SaveQueryInput
    expect(input.title).toBe("SELECT 1")
    expect(input.sql).toBe("SELECT 1")
    expect("folderId" in input).toBe(false)
  })

  it("sends the shared space as null, the way the server reads it", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    const { saveQuery } = renderPage()

    const select = await picker()
    chooseOption(select, "Shared")
    fireEvent.click(screen.getByRole("button", { name: "Save as new" }))

    await waitFor(() => expect(saveQuery).toHaveBeenCalledTimes(1))
    const input = saveQuery.mock.calls[0]?.[0] as SaveQueryInput
    expect(input.folderId).toBeNull()
  })

  it("sends a folder by its id", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    spyOn(queryService, "getFolderAccess").mockResolvedValue(accessList([]))
    const { saveQuery } = renderPage()

    const select = await picker()
    chooseOption(select, "Team")
    fireEvent.click(screen.getByRole("button", { name: "Save as new" }))

    await waitFor(() => expect(saveQuery).toHaveBeenCalledTimes(1))
    const input = saveQuery.mock.calls[0]?.[0] as SaveQueryInput
    expect(input.folderId).toBe("team")
  })

  it("offers no folder and no shared-with-the-team line when the query is being changed", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    spyOn(queryService, "getSaved").mockResolvedValue(SAVED)
    renderPage({ mode: "update", savedId: "q1" })

    await waitFor(() => expect(screen.queryAllByRole("combobox", { name: "Folder" }).length).toBe(0))
    expect(screen.queryAllByText(/shared with the team/i).length).toBe(0)
    expect(screen.getByRole("button", { name: "Save" })).toBeTruthy()
  })

  it("keeps the title and the tags of the saved query for a copy of it", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    spyOn(queryService, "getSaved").mockResolvedValue(SAVED)
    renderPage({ mode: "create", savedId: "q1" })

    await waitFor(() =>
      expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("Orders")
    )
    expect((screen.getByLabelText("Tags") as HTMLInputElement).value).toBe("finance, gold")
  })

  it("starts in the caller's own space on every visit to the page", async () => {
    spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
    spyOn(queryService, "getFolderAccess").mockResolvedValue(accessList([]))
    renderPage()

    const select = await foldersReady()
    chooseOption(select, "Team")
    expect(select.textContent).toContain("Team")

    // The page is left and opened again for the next query: the picker
    // starts in the caller's own space, not where the last visit left it.
    // (This used to be asserted by closing and reopening the sheet; a page
    // is a mount, so the second visit is a second mount.)
    cleanup()
    renderPage()

    const reopened = await foldersReady()
    expect(reopened.textContent).toContain("Personal space (only you)")
    fireEvent.click(reopened)
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual([
      "Personal space (only you)",
      "Personal space (only you) / Reports",
      "Shared",
      "Team",
    ])
  })

  describe("who the chosen folder is shared with", () => {
    /** The line under the picker, once it has loaded. */
    const sharedLine = () => screen.queryAllByText(/^Shared with /)

    /**
     * Wait until the stubbed list has been asked for, resolved and rendered.
     * An absence is only worth asserting after this: before it, "no line"
     * is also what a note that has not loaded yet looks like (QS-3 review).
     */
    async function loaded(spy: { mock: { results: { value: unknown }[] } }) {
      await waitFor(() => expect(spy.mock.results.length).toBeGreaterThan(0))
      await act(async () => {
        await Promise.allSettled(spy.mock.results.map((result) => result.value))
      })
    }

    it("names the people and roles the folder is shared with, without the caller and once each", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(TEAM_ACCESS)
      renderPage()

      chooseOption(await foldersReady(), "Team")

      // Rina (the caller) is left out; Dewi is in the folder's own rules and
      // in an inherited one, and counts once.
      expect(
        await screen.findByText("Shared with 2 people or roles: Dewi Lestari, Finance.")
      ).toBeDefined()
      expect(access.mock.calls.at(-1)?.[0]).toBe("team")
    })

    it("says Unknown person or Unknown role for a grantee the server found no name for", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      spyOn(queryService, "getFolderAccess").mockResolvedValue(
        accessList([rule("u9", null, null), rule(null, "r9", null)])
      )
      renderPage()

      chooseOption(await foldersReady(), "Team")

      expect(
        await screen.findByText("Shared with 2 people or roles: Unknown person, Unknown role.")
      ).toBeDefined()
    })

    it("shows nothing when the caller is the only grantee", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(
        accessList([rule("u1", null, "Rina Wijaya")])
      )
      renderPage()

      chooseOption(await foldersReady(), "Team")

      await loaded(access)
      expect(access).toHaveBeenCalledTimes(1)
      expect(sharedLine().length).toBe(0)
    })

    it("shows nothing when the folder has no rule at all", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(accessList([]))
      renderPage()

      chooseOption(await foldersReady(), "Team")

      await loaded(access)
      expect(access).toHaveBeenCalledTimes(1)
      expect(sharedLine().length).toBe(0)
    })

    it("does not exclude anybody when the console does not know the caller", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      spyOn(queryService, "getFolderAccess").mockResolvedValue(TEAM_ACCESS)
      renderPage({ noViewer: true })

      chooseOption(await foldersReady(), "Team")

      expect(
        await screen.findByText("Shared with 3 people or roles: Rina Wijaya, Dewi Lestari, Finance.")
      ).toBeDefined()
    })

    it("neither asks nor shows anything for the shared space and for the caller's own space", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(TEAM_ACCESS)
      renderPage()

      const select = await foldersReady()
      // Own space is where the picker starts.
      expect(access).not.toHaveBeenCalled()

      chooseOption(select, "Shared")
      // Let any effect the change started run before the count is read.
      await new Promise((resolve) => setTimeout(resolve, 20))

      expect(access).not.toHaveBeenCalled()
      expect(sharedLine().length).toBe(0)
    })

    it("asks for a folder inside the caller's own space, which can be shared", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(
        accessList([rule("u2", null, "Dewi Lestari")])
      )
      renderPage()

      const select = await foldersReady()
      chooseOption(select, "Personal space (only you) / Reports")

      expect(await screen.findByText("Shared with 1 person or role: Dewi Lestari.")).toBeDefined()
      expect(access.mock.calls.at(-1)?.[0]).toBe("reports")
    })

    it("keeps Save as new enabled while the list is still loading", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      const access = spyOn(queryService, "getFolderAccess").mockReturnValue(new Promise(() => {}))
      const { saveQuery } = renderPage()

      chooseOption(await foldersReady(), "Team")
      await waitFor(() => expect(access).toHaveBeenCalledTimes(1))

      const save = screen.getByRole("button", { name: "Save as new" }) as HTMLButtonElement
      expect(save.disabled).toBe(false)
      fireEvent.click(save)
      await waitFor(() => expect(saveQuery).toHaveBeenCalledTimes(1))
    })

    it("shows the server's sentence when the list cannot be read, and still saves", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      spyOn(queryService, "getFolderAccess").mockRejectedValue(
        new ServiceError("unavailable", "unavailable: The folder list could not be read.")
      )
      const { saveQuery } = renderPage()

      chooseOption(await foldersReady(), "Team")

      expect(await screen.findByText("The folder list could not be read.")).toBeDefined()
      expect(sharedLine().length).toBe(0)
      fireEvent.click(screen.getByRole("button", { name: "Save as new" }))
      await waitFor(() => expect(saveQuery).toHaveBeenCalledTimes(1))
      expect((saveQuery.mock.calls[0]?.[0] as SaveQueryInput).folderId).toBe("team")
    })

    it("neither asks nor shows anything for a folder the caller may only view", async () => {
      // The server refuses the access list of a folder the caller holds at
      // view only, so asking would print a refusal for a folder that is
      // simply not the caller's to change (QS-3 review).
      spyOn(queryService, "listFolders").mockResolvedValue([
        ...FOLDERS,
        {
          id: "read-only",
          parentId: null,
          name: "Read only",
          isPersonal: false,
          ownerId: "u2",
          level: "view",
        },
      ])
      const access = spyOn(queryService, "getFolderAccess").mockRejectedValue(
        new ServiceError(
          "permission_denied",
          "permission_denied: You do not have permission to change this folder."
        )
      )
      renderPage()

      const select = await foldersReady()
      chooseOption(select, "Read only")
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 20))
      })

      expect(access).not.toHaveBeenCalled()
      expect(sharedLine().length).toBe(0)
      expect(screen.queryAllByText(/permission/i).length).toBe(0)
    })

    it("does not show the first folder's names for the second while the second loads", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      // What the line says at the moment the second request goes out: the
      // effect that starts it runs right after the commit of the new
      // target, before the hook resets its own state, so a note that kept
      // its instance still shows the first folder's names here (QS-3 review).
      const asked: { lines: number | null } = { lines: null }
      spyOn(queryService, "getFolderAccess").mockImplementation(async (id) => {
        if (id === "team") return TEAM_ACCESS
        asked.lines = sharedLine().length
        return new Promise<SavedQueryAccessList>(() => {})
      })
      renderPage()

      const select = await foldersReady()
      chooseOption(select, "Team")
      expect(
        await screen.findByText("Shared with 2 people or roles: Dewi Lestari, Finance.")
      ).toBeDefined()

      chooseOption(select, "Personal space (only you) / Reports")

      await waitFor(() => expect(asked.lines).not.toBeNull())
      expect(asked.lines).toBe(0)
      expect(sharedLine().length).toBe(0)
    })

    it("neither asks nor shows anything when a query is being changed", async () => {
      spyOn(queryService, "listFolders").mockResolvedValue(FOLDERS)
      spyOn(queryService, "getSaved").mockResolvedValue(SAVED)
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(TEAM_ACCESS)
      renderPage({ mode: "update", savedId: "q1" })

      await waitFor(() => expect(screen.queryAllByRole("combobox", { name: "Folder" }).length).toBe(0))
      await new Promise((resolve) => setTimeout(resolve, 20))

      expect(access).not.toHaveBeenCalled()
      expect(sharedLine().length).toBe(0)
    })
  })
})
