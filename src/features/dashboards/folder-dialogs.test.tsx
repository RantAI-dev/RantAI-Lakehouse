// UX-1-R2: the inline folder-rename input must show its required label so
// the star is visible, instead of hiding it with sr-only.
import { fireEvent, render, screen } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"

import { ManageFoldersDialog } from "./folder-dialogs"

describe("ManageFoldersDialog required label (UX-1-R2)", () => {
  it("keeps the rename input's required label visible and associated", () => {
    render(
      <ManageFoldersDialog
        open
        onOpenChange={mock()}
        folders={[{ id: "f1", name: "Reports", parentId: "" }]}
        onChanged={mock()}
      />
    )

    fireEvent.click(screen.getByLabelText("Rename Reports"))
    const input = screen.getByLabelText("Name")
    expect(input.getAttribute("aria-required")).toBe("true")
    const label = document.querySelector('label[for="folder-rename-name"]')
    expect(label?.getAttribute("data-required")).toBe("")
    expect(label?.className).not.toContain("sr-only")
  })
})
