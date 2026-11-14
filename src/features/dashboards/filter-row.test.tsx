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

const line = () => document.querySelector("[data-layout]") as HTMLElement

// BI-18 part C review fix R5 found the controls wrapping into three rows above
// the tabs at 390 px. BI-61 T3 (layout A's phone) keeps them on one line that
// scrolls sideways instead of hiding them behind a "Filters" button.
describe("FilterRow (BI-18 part C review fix R5, BI-61 T3)", () => {
  it("keeps the chips on one line that scrolls sideways at 390 px, never wrapping", () => {
    atWidth(390)
    render(<FilterRow><button type="button">Add filter</button></FilterRow>)
    expect(screen.getByText("Add filter")).toBeTruthy()
    expect(line().dataset.layout).toBe("scroll")
    expect(line().className).toContain("flex-nowrap")
    expect(line().className).toContain("overflow-x-auto")
    expect(line().className).not.toContain("flex-wrap ")
  })

  it("lets the chips wrap at the right at 900 px", () => {
    atWidth(900)
    render(<FilterRow><button type="button">Add filter</button></FilterRow>)
    expect(screen.getByText("Add filter")).toBeTruthy()
    expect(line().dataset.layout).toBe("wrap")
    expect(line().className).toContain("flex-wrap")
  })

  it("draws no box around the chips", () => {
    atWidth(900)
    render(<FilterRow><span>controls</span></FilterRow>)
    expect(line().className).not.toContain("border")
  })
})
