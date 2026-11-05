// The move dialog of a saved query and of a folder (`QS-3` C2, C6): the
// picker offers the shared space, the caller's own space and the folders
// under them, and the dialog hands the target over as the server reads it:
// an id for a folder, nothing for the shared space.
//
// The service is not called here — the page and the tree are the ones that
// write — so this is a pure component test. A refusal is the server's
// sentence, shown as it was given.

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { QueryFolder } from "@/services/contracts/queries"
import { SavedQueryMoveDialog } from "./saved-query-move-dialog"

afterEach(() => {
  cleanup()
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
    />,
  )
  return { onMove, picker: screen.getByLabelText("Target folder") as HTMLSelectElement }
}

describe("the move dialog", () => {
  it("offers the shared space, the caller's own space and the folders under them", () => {
    const { picker } = renderDialog()

    expect(Array.from(picker.options).map((option) => option.text)).toEqual([
      "Personal space (only you)",
      "Shared",
      "Team",
      "Team / Quarterly",
    ])
  })

  it("hands the shared space over as nothing, the way the server reads it", () => {
    const { onMove, picker } = renderDialog()

    fireEvent.change(picker, { target: { value: "" } })
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    // `null` is the shared space; the console never sends an absent field
    // where the server reads "leave it where it is".
    expect(onMove).toHaveBeenCalledWith(null)
  })

  it("hands a folder over by its id", () => {
    const { onMove, picker } = renderDialog()

    fireEvent.change(picker, { target: { value: "quarterly" } })
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    expect(onMove).toHaveBeenCalledWith("quarterly")
  })

  it("starts on the folder the query sits in", () => {
    const { picker } = renderDialog({ currentFolderId: "team" })

    expect(picker.value).toBe("team")
  })

  it("does not offer the folder that is being moved", () => {
    const { picker } = renderDialog({ excludeId: "team" })

    expect(Array.from(picker.options).map((option) => option.value)).toEqual([
      "personal-1",
      "",
      "quarterly",
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
})
