import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"

import { BoardSwitcher } from "./board-switcher"
import { PrivateBadge } from "./private-badge"

afterEach(cleanup)

const props = {
  boards: [{ id: "b1", name: "Map demo", folderId: "f1" }],
  folders: [{ id: "f1", name: "Tourism", parentId: "" }],
  activeId: "b1",
  activeName: "Map demo",
  onSelect: () => {},
  onCreate: () => {},
}

describe("BoardSwitcher badge (BI-18 part C review fix (SHOULD-FIX) R16)", () => {
  // The browser check of the title row is the owner's; what a test can hold is
  // the structure that put the badge too high: with the folder path above the
  // name, a badge centred on the whole switcher floats above the title.
  it("puts the badge on the name's own line, not centred on the folder path above it", () => {
    render(<BoardSwitcher {...props} badge={<PrivateBadge />} />)
    const name = screen.getByLabelText("Switch dashboard")
    const badge = screen.getByText("Private")
    const row = name.parentElement as HTMLElement
    expect(row.contains(badge)).toBe(true)
    // The row is the line of the name: centred vertically, and the folder path is not in it.
    expect(row.className).toContain("items-center")
    expect(row.contains(screen.getByText("Tourism"))).toBe(false)
  })

  it("shows no badge when none is given", () => {
    render(<BoardSwitcher {...props} />)
    expect(screen.queryByText("Private")).toBeNull()
  })
})
