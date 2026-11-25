import * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import type { BoardTab, LayoutMap } from "@/services"
import { stubRectWidth, stubResizeObserver } from "@/lib/test-layout"
import { DashboardGrid, type GridItem } from "./dashboard-grid"

afterEach(cleanup)

// CI full run 2026-11-15: the canvas width is set here on every prototype that could answer
// (`stubRectWidth`), not on `Element.prototype` alone, which a spy left over by another file in the
// same bun process (copilot-dock.test.tsx) shadowed; and undone after each test.
let undo: (() => void)[] = []
function atWidth(width: number) {
  undo.push(stubRectWidth(() => width))
}
beforeEach(() => {
  undo.push(stubResizeObserver())
  try { window.localStorage.clear() } catch { /* no storage */ }
})
afterEach(() => {
  for (const u of undo.reverse()) u()
  undo = []
})

const item = (id: string, title: string, kind?: string): GridItem => ({ id, kind, title, body: <p>{`body of ${title}`}</p> })
const items = [item("a", "Alpha"), item("b", "Bravo"), item("s", "Region details", "section"), item("c", "Charlie"), item("h", "A heading", "heading")]
const layout: LayoutMap = {
  a: { x: 0, y: 0, w: 6, h: 4, tab: "t1" },
  b: { x: 6, y: 0, w: 6, h: 4, tab: "t1" },
  s: { x: 0, y: 4, w: 12, h: 1, tab: "t1" },
  c: { x: 0, y: 5, w: 12, h: 4, tab: "t1" },
  h: { x: 0, y: 0, w: 12, h: 1, tab: "t2" },
}
const tabs: BoardTab[] = [{ id: "t1", title: "One" }, { id: "t2", title: "Two" }]

function grid(props: Partial<React.ComponentProps<typeof DashboardGrid>> = {}) {
  render(<DashboardGrid items={items} layout={layout} editable={false} onLayoutChange={() => {}} tabs={tabs} tab="t1" foldKey="b1" {...props} />)
}

describe("dashboard grid with tabs and sections", () => {
  it("draws only the tiles of the selected tab", () => {
    atWidth(1200)
    grid()
    expect(screen.getByText("Alpha")).toBeDefined()
    expect(screen.queryByText("A heading")).toBeNull()
    cleanup()
    grid({ tab: "t2" })
    expect(screen.getByText("A heading")).toBeDefined()
    expect(screen.queryByText("Alpha")).toBeNull()
  })

  it("draws every tile when the board has no tabs", () => {
    atWidth(1200)
    grid({ tabs: undefined, tab: undefined })
    expect(screen.getByText("Alpha")).toBeDefined()
    expect(screen.getByText("A heading")).toBeDefined()
  })

  it("folds the tiles below a section header, for the viewer only", () => {
    atWidth(1200)
    grid()
    expect(screen.getByText("body of Charlie")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: /Region details/ }))
    expect(screen.queryByText("body of Charlie")).toBeNull()
    // Tiles above the section stay.
    expect(screen.getByText("body of Alpha")).toBeDefined()
    expect(window.localStorage.getItem("dash-folded:b1")).toBe(JSON.stringify(["s"]))
    fireEvent.click(screen.getByRole("button", { name: /Region details/ }))
    expect(screen.getByText("body of Charlie")).toBeDefined()
  })

  it("does not fold in edit mode", () => {
    atWidth(1200)
    window.localStorage.setItem("dash-folded:b1", JSON.stringify(["s"]))
    grid({ editable: true })
    expect(screen.getByText("body of Charlie")).toBeDefined()
  })

  it("stacks the tiles in one column under 640 px, in reading order", () => {
    atWidth(390)
    grid()
    const stack = screen.getByTestId("phone-stack")
    const titles = Array.from(stack.querySelectorAll("p.truncate, button span.truncate")).map((n) => n.textContent)
    expect(titles.slice(0, 4)).toEqual(["Alpha", "Bravo", "Region details", "Charlie"])
  })

  it("uses the phone layout instead of the reading order when there is one", () => {
    atWidth(390)
    grid({ phone: [{ id: "c", h: 3 }, { id: "a", h: 5 }] })
    const stack = screen.getByTestId("phone-stack")
    const titles = Array.from(stack.querySelectorAll("p.truncate, button span.truncate")).map((n) => n.textContent)
    // The tiles it names first, then the rest in reading order.
    expect(titles).toEqual(["Charlie", "Alpha", "Bravo", "Region details"])
  })

  it("keeps the desktop canvas at 640 px and wider", () => {
    atWidth(700)
    grid()
    expect(screen.queryByTestId("phone-stack")).toBeNull()
  })

  it("edits the phone layout at phone width whatever the window, and reports the whole new list", () => {
    atWidth(1200)
    let saved: { id: string; h: number }[] = []
    grid({ editable: true, phoneEdit: true, onPhoneChange: (next) => { saved = next } })
    expect(screen.getByTestId("phone-stack")).toBeDefined()
    fireEvent.click(screen.getByLabelText("Move Bravo up"))
    expect(saved.map((p) => p.id)).toEqual(["b", "a", "s", "c"])
    fireEvent.click(screen.getByLabelText("Make Alpha taller"))
    expect(saved.find((p) => p.id === "a")?.h).toBe(5)
  })
})

// Owner QA 2026-11-15: reading, a canvas that squeezes tiles under 240 px lays that row out again
// (2 + 2 for four 3-column KPIs at 900 px); edit mode keeps the saved grid. Small-by-design tiles
// stay as authored (BI-61 review, reflow SHOULD-FIX 1).
describe("dashboard grid reflow on a narrow canvas", () => {
  const kpiItems = ["k1", "k2", "k3", "k4"].map((id) => item(id, id.toUpperCase()))
  const kpiLayout: LayoutMap = Object.fromEntries(
    ["k1", "k2", "k3", "k4"].map((id, i) => [id, { x: i * 3, y: 0, w: 3, h: 3 }]),
  )
  const boxOf = (title: string) => {
    let el: HTMLElement | null = screen.getByText(`body of ${title}`)
    while (el && !el.classList.contains("dash-tile")) el = el.parentElement
    return el!.style
  }

  it("draws a row of four three-column tiles two a row at 900 px, without saving anything", () => {
    atWidth(900)
    let saved = false
    render(<DashboardGrid items={kpiItems} layout={kpiLayout} editable={false} onLayoutChange={() => { saved = true }} />)
    // 900 px: a column is 64 px, so half the canvas is 6 columns (444 px) and a row is 56 px.
    expect(boxOf("K1").left).toBe("0px")
    expect(boxOf("K2").left).toBe("456px")
    expect(boxOf("K3").top).toBe("168px")
    expect(boxOf("K3").width).toBe("444px")
    expect(saved).toBe(false)
  })

  it("keeps the saved grid in edit mode", () => {
    atWidth(900)
    render(<DashboardGrid items={kpiItems} layout={kpiLayout} editable onLayoutChange={() => {}} />)
    expect(boxOf("K2").left).toBe("228px")
    expect(boxOf("K3").top).toBe("0px")
    expect(boxOf("K3").width).toBe("216px")
  })

  it("keeps six 2-column KPIs as authored on a 1440 px canvas (small by design)", () => {
    atWidth(1440)
    const six = ["k1", "k2", "k3", "k4", "k5", "k6"].map((id) => item(id, id.toUpperCase()))
    const sixLayout: LayoutMap = Object.fromEntries(six.map((it, i) => [it.id, { x: i * 2, y: 0, w: 2, h: 3 }]))
    render(<DashboardGrid items={six} layout={sixLayout} editable={false} onLayoutChange={() => {}} />)
    for (const t of ["K1", "K6"]) expect(boxOf(t).top).toBe("0px")
  })

  it("keeps the saved grid when every tile is wide enough", () => {
    atWidth(1350)
    render(<DashboardGrid items={kpiItems} layout={kpiLayout} editable={false} onLayoutChange={() => {}} />)
    expect(boxOf("K4").top).toBe("0px")
  })
})

describe("edit mode canvas", () => {
  it("keeps every resize handle inside its tile box, so the scroll region never widens", () => {
    atWidth(1200)
    grid({ editable: true, tabs: undefined, tab: undefined })
    const handles = document.querySelectorAll('[title="Drag to resize"]')
    expect(handles.length).toBeGreaterThan(0)
    for (const h of Array.from(handles)) {
      const cls = h.className
      expect(cls).not.toMatch(/-(right|bottom|left|top)-\d/)
      expect(cls).toMatch(/\bbottom-\d/)
      expect(cls).toMatch(/\bright-\d/)
    }
  })
})
