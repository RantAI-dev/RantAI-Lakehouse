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
  menuNote: { label: "12:47 · mart_wisman", detail: "Data from 12:47, remembered. Reads mart_wisman." },
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

  // Owner QA 2026-11-15: the chip covered a narrow tile's title. It now sits beside the title (in the
  // header's flow, not laid over it) and shows on hover only on a tile at least 360 px wide (a
  // container query on the tile, so a side panel narrowing the page counts). Was: an absolute chip
  // faded in over the title on any tile from the `lg` window width.
  it("keeps the data time for the hover chip, beside the title and only on a wide enough tile", () => {
    render(<TileCard item={item()} editable={false} look="soft" />)
    const tile = document.querySelector("[data-slot='tile']") as HTMLElement
    expect(tile.className).toContain("@container/tile")
    const line = document.querySelector("[data-slot='tile-data-from']") as HTMLElement
    expect(line.textContent).toBe("Data from 12:47")
    expect(line.className).not.toContain("absolute")
    expect(line.className).toMatch(/(^|\s)hidden(\s|$)/)
    expect(line.className).toContain("@min-[22.5rem]/tile:group-hover/tile:inline")
    expect(line.className).toContain("@min-[22.5rem]/tile:group-focus-within/tile:inline")
  })

  it("gives a cut title and description a tooltip and a tab stop, and a title that fits neither", () => {
    const realScroll = Object.getOwnPropertyDescriptor(Element.prototype, "scrollWidth")
    const realClient = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth")
    try {
      // Every line is cut: 300 px of text in 100 px.
      Object.defineProperty(Element.prototype, "scrollWidth", { configurable: true, get: () => 300 })
      Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 100 })
      render(<TileCard item={item({ subtitle: "Foreign visitors by year since 2015" })} editable={false} look="soft" />)
      expect(screen.getByText("Visitors per year").getAttribute("tabindex")).toBe("0")
      expect(screen.getByText("Foreign visitors by year since 2015").getAttribute("tabindex")).toBe("0")
    } finally {
      if (realScroll) Object.defineProperty(Element.prototype, "scrollWidth", realScroll)
      if (realClient) Object.defineProperty(HTMLElement.prototype, "clientWidth", realClient)
    }
    cleanup()
    render(<TileCard item={item()} editable={false} look="soft" />)
    expect(screen.getByText("Visitors per year").getAttribute("tabindex")).toBeNull()
  })

  // BI-61 review round 1, SHOULD-FIX 3: one short line that never wraps, the full sentence as its tooltip.
  it("puts one short line with the data time and the source at the top of its ⋯, and the downloads in a submenu", async () => {
    render(<TileCard item={item()} editable={false} look="soft" />)
    fireEvent.click(screen.getByRole("button", { name: "Actions for Visitors per year" }))
    const note = await screen.findByText("12:47 · mart_wisman")
    expect(note.getAttribute("data-slot")).toBe("tile-menu-notes")
    expect(note.tagName).toBe("P")
    expect(note.className).toContain("truncate")
    expect(note.getAttribute("title")).toBe("Data from 12:47, remembered. Reads mart_wisman.")
    expect(document.querySelectorAll("[data-slot='tile-menu-notes']")).toHaveLength(1)
    expect(screen.queryByText(/auto-refresh|made by hand/)).toBeNull()
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
