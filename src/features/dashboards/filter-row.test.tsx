import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"

import { stubRectWidth, stubResizeObserver } from "@/lib/test-layout"
import { FilterRow } from "./filter-row"

// CI full run 2026-11-15: the width is set on every prototype that could answer (`test-layout.ts`),
// so a spy another file left on `HTMLElement.prototype` cannot hide it, and undone after each test.
let undo: (() => void)[] = []
function atWidth(width: number) {
  undo.push(stubRectWidth(() => width), stubResizeObserver())
}

afterEach(() => {
  for (const u of undo.reverse()) u()
  undo = []
  cleanup()
})

describe("FilterRow (BI-18 part C review fix R5)", () => {
  it("collapses to one Filters button with the count at 390 px and keeps the controls out of the page", () => {
    atWidth(390)
    render(<FilterRow count={2}><button type="button">Add filter</button></FilterRow>)
    expect(screen.getByRole("button", { name: /Filters · 2/ })).toBeTruthy()
    expect(screen.queryByText("Add filter")).toBeNull()
  })

  it("says just Filters when nothing is set", () => {
    atWidth(390)
    render(<FilterRow count={0}><span>controls</span></FilterRow>)
    expect(screen.getByRole("button", { name: "Filters" })).toBeTruthy()
  })

  it("renders the controls inline at 900 px", () => {
    atWidth(900)
    render(<FilterRow count={2}><button type="button">Add filter</button></FilterRow>)
    expect(screen.getByText("Add filter")).toBeTruthy()
    expect(screen.queryByRole("button", { name: /Filters/ })).toBeNull()
  })
})
