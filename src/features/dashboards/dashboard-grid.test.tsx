import * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import type { BoardTab, LayoutMap } from "@/services/clients/bi-store"
import { DashboardGrid, type GridItem } from "./dashboard-grid"

afterEach(cleanup)

const realRect = Element.prototype.getBoundingClientRect
function atWidth(width: number) {
  Element.prototype.getBoundingClientRect = function () {
    return { width, height: 0, top: 0, left: 0, right: width, bottom: 0, x: 0, y: 0, toJSON: () => ({}) } as DOMRect
  }
}
beforeEach(() => {
  ;(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class { observe() {} disconnect() {} unobserve() {} }
  try { window.localStorage.clear() } catch { /* no storage */ }
})
afterEach(() => { Element.prototype.getBoundingClientRect = realRect })

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
