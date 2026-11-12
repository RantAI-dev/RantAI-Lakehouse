// The move dialog of a saved query and of a folder (`QS-3` C2, C6): the
// picker offers the shared space, the caller's own space and the folders
// under them, and the dialog hands the target over as the server reads it:
// an id for a folder, nothing for the shared space.
//
// The dialog never writes — the page and the tree do — but it reads: under
// the chosen target it asks the folder's access list and says who the folder
// is shared with. That one read goes through `queryService.getFolderAccess`,
// which the tests spy, so nothing leaves the test. A refusal is the server's
// sentence, shown as it was given.

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { queryService } from "@/services"
import type { QueryFolder, SavedQueryAccessList } from "@/services/contracts/queries"
import { ServiceError } from "@/services/errors"
import { SavedQueryMoveDialog } from "./saved-query-move-dialog"

afterEach(() => {
  cleanup()
  mock.restore()
})

/** The caller of these tests: the person who moves, never named in the line. */
const VIEWER = "u1"

function rule(
  userId: string | null,
  roleId: string | null,
  granteeName: string | null,
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

/** A folder's access list: its own rules, then the rules it inherits. */
function accessList(
  rules: SavedQueryAccessList["rules"],
  inherited: SavedQueryAccessList["inherited"] = [],
): SavedQueryAccessList {
  return { itemType: "folder", itemId: "team", yourLevel: "edit", rules, inherited }
}

/** Dewi twice (own and inherited) counts once; the caller is left out. */
const TEAM_ACCESS = accessList(
  [rule("u1", null, "Rina Wijaya"), rule("u2", null, "Dewi Lestari"), rule(null, "r1", "Finance")],
  [
    {
      folderId: "root-team",
      folderName: "Company",
      userId: "u2",
      roleId: null,
      granteeName: "Dewi Lestari",
      granteeEmail: null,
      level: "view",
    },
  ],
)

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

function renderDialog(over: {
  readonly currentFolderId?: string | null
  readonly excludeId?: string
  readonly error?: string | null
  readonly onMove?: (target: string | null) => void
  readonly noViewer?: boolean
} = {}) {
  const onMove = over.onMove ?? mock(() => {})
  render(
    <SavedQueryMoveDialog
      folders={FOLDERS}
      subject="Revenue"
      currentFolderId={over.currentFolderId ?? null}
      excludeId={over.excludeId}
      onClose={() => {}}
      onMove={onMove}
      error={over.error ?? null}
      viewerId={over.noViewer ? undefined : VIEWER}
    />,
  )
  return { onMove, picker: screen.getByRole("combobox", { name: "Target folder" }) }
}

describe("the move dialog", () => {
  it("offers the shared space, the caller's own space and the folders under them", () => {
    const { picker } = renderDialog()

    fireEvent.click(picker)
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual([
      "Personal space (only you)",
      "Shared",
      "Team",
      "Team / Quarterly",
    ])
  })

  it("hands the shared space over as nothing, the way the server reads it", () => {
    const { onMove, picker } = renderDialog()

    chooseOption(picker, "Shared")
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    // `null` is the shared space; the console never sends an absent field
    // where the server reads "leave it where it is".
    expect(onMove).toHaveBeenCalledWith(null)
  })

  it("hands a folder over by its id", () => {
    spyOn(queryService, "getFolderAccess").mockResolvedValue(accessList([]))
    const { onMove, picker } = renderDialog()

    chooseOption(picker, "Team / Quarterly")
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    expect(onMove).toHaveBeenCalledWith("quarterly")
  })

  it("starts on the folder the query sits in", () => {
    spyOn(queryService, "getFolderAccess").mockResolvedValue(accessList([]))
    const { picker } = renderDialog({ currentFolderId: "team" })

    expect(picker.textContent).toContain("Team")
  })

  it("does not offer the folder that is being moved", () => {
    const { picker } = renderDialog({ excludeId: "team" })

    fireEvent.click(picker)
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual([
      "Personal space (only you)",
      "Shared",
      "Team / Quarterly",
    ])
  })

  it("shows the server's sentence when the move is refused", () => {
    renderDialog({
      error:
        "permission_denied: Only someone who manages this can move it.",
    })

    // The server's own sentence, word for word: the console writes none of
    // its own (C6).
    expect(screen.getByRole("alert").textContent).toBe(
      "Only someone who manages this can move it.",
    )
  })
  describe("who the chosen folder is shared with", () => {
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
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(TEAM_ACCESS)
      const { picker } = renderDialog()

      chooseOption(picker, "Team")

      expect(await screen.findByText("Shared with 2 people or roles: Dewi Lestari, Finance.")).toBeDefined()
      expect(access.mock.calls.at(-1)?.[0]).toBe("team")
    })

    it("asks for the folder the dialog opens on", async () => {
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(TEAM_ACCESS)
      renderDialog({ currentFolderId: "team" })

      expect(await screen.findByText("Shared with 2 people or roles: Dewi Lestari, Finance.")).toBeDefined()
      expect(access.mock.calls.at(-1)?.[0]).toBe("team")
    })

    it("says Unknown person or Unknown role for a grantee the server found no name for", async () => {
      spyOn(queryService, "getFolderAccess").mockResolvedValue(
        accessList([rule("u9", null, null), rule(null, "r9", null)]),
      )
      const { picker } = renderDialog()

      chooseOption(picker, "Team")

      expect(
        await screen.findByText("Shared with 2 people or roles: Unknown person, Unknown role."),
      ).toBeDefined()
    })

    it("shows nothing when the caller is the only grantee", async () => {
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(
        accessList([rule("u1", null, "Rina Wijaya")]),
      )
      const { picker } = renderDialog()

      chooseOption(picker, "Team")

      await loaded(access)
      expect(access).toHaveBeenCalledTimes(1)
      expect(sharedLine().length).toBe(0)
    })

    it("shows nothing when the folder has no rule at all", async () => {
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(accessList([]))
      const { picker } = renderDialog()

      chooseOption(picker, "Team")

      await loaded(access)
      expect(access).toHaveBeenCalledTimes(1)
      expect(sharedLine().length).toBe(0)
    })

    it("does not exclude anybody when the console does not know the caller", async () => {
      spyOn(queryService, "getFolderAccess").mockResolvedValue(TEAM_ACCESS)
      const { picker } = renderDialog({ noViewer: true })

      chooseOption(picker, "Team")

      expect(
        await screen.findByText("Shared with 3 people or roles: Rina Wijaya, Dewi Lestari, Finance."),
      ).toBeDefined()
    })

    it("neither asks nor shows anything for the shared space and for the caller's own space", async () => {
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(TEAM_ACCESS)
      // The dialog opens on the shared space (value "").
      const { picker } = renderDialog({ currentFolderId: null })

      chooseOption(picker, "Personal space (only you)")
      await new Promise((resolve) => setTimeout(resolve, 20))

      expect(access).not.toHaveBeenCalled()
      expect(sharedLine().length).toBe(0)
    })

    it("asks for a personal sub-folder, which can be shared", async () => {
      const access = spyOn(queryService, "getFolderAccess").mockResolvedValue(
        accessList([rule("u2", null, "Dewi Lestari")]),
      )
      render(
        <SavedQueryMoveDialog
          folders={[
            ...FOLDERS,
            {
              id: "reports",
              parentId: "personal-1",
              name: "Reports",
              isPersonal: false,
              ownerId: "u1",
              level: "manage",
            },
          ]}
          subject="Revenue"
          currentFolderId={null}
          onClose={() => {}}
          onMove={() => {}}
          viewerId={VIEWER}
        />,
      )

      chooseOption(screen.getByRole("combobox", { name: "Target folder" }), "Personal space (only you) / Reports")

      expect(await screen.findByText("Shared with 1 person or role: Dewi Lestari.")).toBeDefined()
      expect(access.mock.calls.at(-1)?.[0]).toBe("reports")
    })

    it("keeps Move enabled while the list is still loading", async () => {
      const access = spyOn(queryService, "getFolderAccess").mockReturnValue(new Promise(() => {}))
      const { picker, onMove } = renderDialog()

      chooseOption(picker, "Team")
      await waitFor(() => expect(access).toHaveBeenCalledTimes(1))

      const move = screen.getByRole("button", { name: "Move" }) as HTMLButtonElement
      expect(move.disabled).toBe(false)
      fireEvent.click(move)
      expect(onMove).toHaveBeenCalledWith("team")
    })

    it("shows the server's sentence when the list cannot be read, and still moves", async () => {
      spyOn(queryService, "getFolderAccess").mockRejectedValue(
        new ServiceError("unavailable", "unavailable: The folder list could not be read."),
      )
      const { picker, onMove } = renderDialog()

      chooseOption(picker, "Team")

      expect(await screen.findByText("The folder list could not be read.")).toBeDefined()
      expect(sharedLine().length).toBe(0)
      fireEvent.click(screen.getByRole("button", { name: "Move" }))
      expect(onMove).toHaveBeenCalledWith("team")
    })

    it("neither asks nor shows anything for a folder the caller may only view", async () => {
      // The dialog offers every folder, the one the query sits in included,
      // but the server refuses the access list of a folder held at view
      // only; asking would print a refusal for it (QS-3 review).
      const access = spyOn(queryService, "getFolderAccess").mockRejectedValue(
        new ServiceError("permission_denied", "permission_denied: You do not have permission to change this folder."),
      )
      render(
        <SavedQueryMoveDialog
          folders={[
            ...FOLDERS,
            {
              id: "read-only",
              parentId: null,
              name: "Read only",
              isPersonal: false,
              ownerId: "u2",
              level: "view",
            },
          ]}
          subject="Revenue"
          currentFolderId="read-only"
          onClose={() => {}}
          onMove={() => {}}
          viewerId={VIEWER}
        />,
      )

      // Opening on the folder the query sits in asks nothing.
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 20))
      })
      expect(access).not.toHaveBeenCalled()
      expect(sharedLine().length).toBe(0)
      expect(screen.queryAllByText(/permission/i).length).toBe(0)

      chooseOption(screen.getByRole("combobox", { name: "Target folder" }), "Read only")
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 20))
      })
      expect(access).not.toHaveBeenCalled()
    })

    it("does not show the first folder's names for the second while the second loads", async () => {
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
      const { picker } = renderDialog({ currentFolderId: "team" })
      expect(await screen.findByText("Shared with 2 people or roles: Dewi Lestari, Finance.")).toBeDefined()

      chooseOption(picker, "Team / Quarterly")

      await waitFor(() => expect(asked.lines).not.toBeNull())
      expect(asked.lines).toBe(0)
      expect(sharedLine().length).toBe(0)
    })
  })
})
