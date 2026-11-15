import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import { KpiValue } from "./fit-text"

// Owner QA 2026-11-15: a KPI number is never cut; too wide even when smaller, it turns compact
// and gives the full value to the tooltip and to screen readers.
const realRect = Element.prototype.getBoundingClientRect
const realClient = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth")

/** Every text is 20 px a character at the base size; the number's box is `room` px wide. */
function atRoom(room: number) {
  Element.prototype.getBoundingClientRect = function (this: Element) {
    const width = (this.textContent ?? "").length * 20
    return { width, height: 0, top: 0, left: 0, right: width, bottom: 0, x: 0, y: 0, toJSON: () => ({}) } as DOMRect
  }
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => room })
}

beforeEach(() => {
  ;(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class { observe() {} disconnect() {} unobserve() {} }
})
afterEach(() => {
  cleanup()
  Element.prototype.getBoundingClientRect = realRect
  if (realClient) Object.defineProperty(HTMLElement.prototype, "clientWidth", realClient)
})

describe("KpiValue", () => {
  it("shows the full number when it fits", () => {
    atRoom(400)
    render(<div><KpiValue full="13,330,172" compact="13.3M" className="text-4xl" /></div>)
    const n = screen.getByText("13,330,172")
    expect(n.tagName).toBe("P")
    expect(n.getAttribute("tabindex")).toBeNull()
    expect(n.style.fontSize).toBe("")
    expect(screen.queryByText("13.3M")).toBeNull()
  })

  it("keeps the full number, smaller, while the floor allows", () => {
    atRoom(161)
    render(<div><KpiValue full="13,330,172" compact="13.3M" className="text-4xl" /></div>)
    // 160 px of room for 200 px of text: four fifths of the base size (the test DOM's 16 px, no stylesheet).
    expect(Number.parseFloat(screen.getByText("13,330,172").style.fontSize)).toBeCloseTo(16 * 0.8)
    expect(screen.queryByText("13.3M")).toBeNull()
  })

  it("turns compact below the floor, with the full value for screen readers and a focusable tooltip", () => {
    atRoom(101)
    render(<div><KpiValue full="13,330,172" compact="13.3M" className="text-4xl" /></div>)
    const short = screen.getByText("13.3M")
    expect(short.getAttribute("aria-hidden")).toBe("true")
    const full = screen.getByText("13,330,172")
    expect(full.className).toContain("sr-only")
    expect(short.parentElement?.getAttribute("tabindex")).toBe("0")
  })

  it("leaves no measuring copy behind", () => {
    atRoom(101)
    const { container } = render(<div><KpiValue full="13,330,172" compact="13.3M" className="text-4xl" /></div>)
    expect(container.querySelectorAll("p")).toHaveLength(1)
  })
})
