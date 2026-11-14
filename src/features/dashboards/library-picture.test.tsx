import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"

import { boardItem, reportItem, templateItem } from "./library-items"
import { BoardPicture, ItemPicture, boardTiles, chartsOf, pictureSource } from "./library-picture"

afterEach(cleanup)

// BI-59 T3 (decision P3): pictures drawn from chart definitions, never from a query.

const charts = [
  { id: "u1", board: "b1", kind: "kpi" },
  { id: "u2", board: "b1", kind: "bar" },
  { id: "u3", board: "b1", kind: "line", span: 2 as const },
  { id: "u4", board: "b2", kind: "pie" },
]

describe("boardTiles", () => {
  it("keeps the saved boxes, places a chart without one by the shared rule, and reads top to bottom", () => {
    const tiles = boardTiles({ layout: { u2: { x: 6, y: 2, w: 6, h: 6 }, u1: { x: 0, y: 2, w: 6, h: 3 } } }, charts.slice(0, 3))
    expect(tiles.map((t) => [t.id, t.x, t.y, t.w])).toEqual([
      ["u1", 0, 0, 6],
      ["u2", 6, 0, 6],
      ["u3", 0, 6, 12],
    ])
  })

  it("draws only the first tab of a tabbed board", () => {
    const tiles = boardTiles(
      { tabs: [{ id: "t1", title: "One" }, { id: "t2", title: "Two" }], layout: { u1: { x: 0, y: 0, w: 6, h: 3, tab: "t2" }, u2: { x: 0, y: 0, w: 6, h: 6, tab: "t1" } } },
      charts.slice(0, 2),
    )
    expect(tiles.map((t) => t.id)).toEqual(["u2"])
  })
})

describe("the pictures", () => {
  it("draws one tile per chart in view, with no number in it", () => {
    const { container } = render(<BoardPicture board={{ layout: {} }} charts={charts.slice(0, 3)} size="lg" />)
    expect(container.querySelector("svg")).toBeTruthy()
    expect(container.querySelectorAll("svg > g").length).toBe(3)
    expect(container.textContent).toBe("")
  })

  it("gives the built-in board, an empty board and an unknown read a neutral picture, not invented shapes", () => {
    render(<ItemPicture item={boardItem({ id: "default", name: "Main", builtin: true })} charts={[]} size="lg" />)
    expect(screen.getByText("Built-in dashboard")).toBeTruthy()
    cleanup()
    render(<ItemPicture item={boardItem({ id: "b9", name: "Empty" })} charts={[]} size="lg" />)
    expect(screen.getByText("No tiles yet")).toBeTruthy()
    cleanup()
    const { container } = render(<ItemPicture item={boardItem({ id: "b1", name: "QA" })} charts={null} size="lg" />)
    // A glyph only: no drawing of tiles.
    expect(container.querySelector('svg[role="presentation"]')).toBeNull()
  })

  it("draws a template as its count and a report as a page with its real title", () => {
    render(<ItemPicture item={templateItem({ id: "t", name: "T", sourceBoard: "", sourceBoardName: "", chartCount: 3, createdBy: "", createdAt: "" })} charts={null} size="lg" />)
    expect(screen.getByText("3 charts")).toBeTruthy()
    cleanup()
    render(<ItemPicture item={reportItem({ id: "r", title: "Wisman 2020-2024", folderId: "", createdBy: "", updatedBy: "", updatedAt: "", blockCount: 2, chartCount: 1 })} charts={null} size="md" />)
    expect(screen.getByText("Wisman 2020-2024")).toBeTruthy()
  })

  it("groups the one read by board: none is [], a failed read is null", () => {
    const source = pictureSource(charts)
    expect(chartsOf(source, "b1")?.map((c) => c.id)).toEqual(["u1", "u2", "u3"])
    expect(chartsOf(source, "b9")).toEqual([])
    expect(chartsOf(pictureSource(null), "b1")).toBeNull()
  })
})
