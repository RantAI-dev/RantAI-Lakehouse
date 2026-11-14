import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { GridItem } from "./dashboard-grid"
import { TileCard } from "./tile-card"

afterEach(cleanup)

const item = (over: Partial<GridItem> = {}): GridItem => ({
  id: "c_1",
  title: "Visitors per year",
  body: <div>chart</div>,
  badge: <span>Manual</span>,
  menuNotes: ["Data from 12:47 · auto-refresh off", "Reads mart_wisman · made by hand"],
  dataFrom: "Data from 12:47",
  menu: [
    { label: "Edit chart", icon: null, onSelect: () => {} },
    {
      label: "Download", icon: null, onSelect: () => {},
      sub: [{ label: "Download data…", icon: null, onSelect: () => {} }, { label: "Download PNG", icon: null, onSelect: () => {} }],
    },
  ],
  ...over,
})

// BI-61 T4 (spec T3, AC3): the dashboard page's tiles have a soft fill, no
// border and no badge; the data time shows on hover, and the ⋯ holds the
// refresh mode and the downloads.
describe("TileCard, the dashboard page's soft look (BI-61 T4)", () => {
  it("draws no border and no badge", () => {
    render(<TileCard item={item()} editable={false} look="soft" />)
    const tile = document.querySelector("[data-slot='tile']") as HTMLElement
    expect(tile.className).not.toMatch(/(^|\s)border(\s|$)/)
    expect(screen.queryByText("Manual")).toBeNull()
  })

  it("keeps the data time for the hover line", () => {
    render(<TileCard item={item()} editable={false} look="soft" />)
    const line = document.querySelector("[data-slot='tile-data-from']") as HTMLElement
    expect(line.textContent).toBe("Data from 12:47")
    expect(line.className).toContain("group-hover/tile:opacity-100")
  })

  it("puts the data time, the refresh mode and the source at the top of its ⋯, and the downloads in a submenu", async () => {
    render(<TileCard item={item()} editable={false} look="soft" />)
    fireEvent.click(screen.getByRole("button", { name: "Actions for Visitors per year" }))
    const notes = await screen.findByText("Data from 12:47 · auto-refresh off")
    expect(notes.closest("[data-slot='tile-menu-notes']")?.textContent).toContain("Reads mart_wisman · made by hand")
    fireEvent.click(screen.getByText("Download"))
    expect(await screen.findByText("Download data…")).toBeTruthy()
    expect(screen.getByText("Download PNG")).toBeTruthy()
  })

  it("runs a submenu entry", async () => {
    const onPng = mock(() => {})
    const it = item()
    it.menu![1].sub![1].onSelect = onPng
    render(<TileCard item={it} editable={false} look="soft" />)
    fireEvent.click(screen.getByRole("button", { name: "Actions for Visitors per year" }))
    fireEvent.click(await screen.findByText("Download"))
    fireEvent.click(await screen.findByText("Download PNG"))
    expect(onPng).toHaveBeenCalledTimes(1)
  })

  it("keeps the bordered card with its badge for the embed and the public link", () => {
    render(<TileCard item={item()} editable={false} />)
    expect(screen.getByText("Manual")).toBeTruthy()
  })
})
