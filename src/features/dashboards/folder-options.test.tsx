import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"

import { MoveBoardDialog } from "./folder-dialogs"
import { PERSONAL_OPTION_LABEL, folderOptionText, splitFolderOptions } from "@/lib/dashboard-organise"

afterEach(cleanup)

// The picker is handed the folders the caller may file into: the shared ones and
// his own personal space (named "Personal space" by `labelledFolders`).
const folders = [
  { id: "f_1", name: "Sales", parentId: "" },
  { id: "f_2", name: "Q1", parentId: "f_1" },
  { id: "fp_me", name: "Personal space", parentId: "", ownerId: "me" },
  { id: "f_3", name: "Ops", parentId: "" },
]

describe("folder pickers (BI-18 part C review fix (SHOULD-FIX) R17)", () => {
  it("lists the personal space first, labelled as only yours, then no folder, then the shared folders", () => {
    render(
      <MoveBoardDialog open onOpenChange={() => {}} folders={folders} boardId="b1" currentFolderId="" onMoved={() => {}} />,
    )
    fireEvent.click(screen.getAllByRole("combobox")[0])
    const options = screen.getAllByRole("option").map((o) => o.textContent)
    expect(options).toEqual([PERSONAL_OPTION_LABEL, "Top level (no folder)", "Ops", "Sales", "Sales / Q1"])
    expect(PERSONAL_OPTION_LABEL).toBe("Personal space (only you)")
  })

  it("splits the personal space from the shared folders and names the chosen value the same way", () => {
    const flat = folders.map((folder) => ({ folder }))
    const { personal, shared } = splitFolderOptions(flat, folders)
    expect(personal.map((o) => o.folder.id)).toEqual(["fp_me"])
    expect(shared.map((o) => o.folder.id)).toEqual(["f_1", "f_2", "f_3"])
    expect(folderOptionText(folders, "fp_me", "No folder")).toBe(PERSONAL_OPTION_LABEL)
    expect(folderOptionText(folders, "f_2", "No folder")).toBe("Sales / Q1")
    expect(folderOptionText(folders, "", "No folder")).toBe("No folder")
  })
})
