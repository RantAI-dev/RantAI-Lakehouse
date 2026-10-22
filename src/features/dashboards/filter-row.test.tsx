import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"

import { FilterRow } from "./filter-row"

const realRect = Element.prototype.getBoundingClientRect
function atWidth(width: number) {
  Element.prototype.getBoundingClientRect = function () {
    return { width, height: 0, top: 0, left: 0, right: width, bottom: 0, x: 0, y: 0, toJSON: () => ({}) } as DOMRect
  }
  ;(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class { observe() {} disconnect() {} unobserve() {} }
}

afterEach(() => {
  Element.prototype.getBoundingClientRect = realRect
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
