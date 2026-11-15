import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import { stubClientWidth, stubRectWidth, stubResizeObserver } from "@/lib/test-layout"
import { KpiValue } from "./fit-text"

// Owner QA 2026-11-15: a KPI number is never cut; too wide even when smaller, it turns compact
// and gives the full value to the tooltip and to screen readers.
// CI full run 2026-11-15: widths are set on every prototype that could answer (`test-layout.ts`),
// so a spy another file left on `HTMLElement.prototype` cannot hide them, and undone after each test.
let undo: (() => void)[] = []

/** Every text is 20 px a character at the base size; the number's box is `room` px wide. */
function atRoom(room: number) {
  undo.push(stubRectWidth((el) => (el.textContent ?? "").length * 20), stubClientWidth(() => room))
}

beforeEach(() => {
  undo.push(stubResizeObserver())
})
afterEach(() => {
  cleanup()
  for (const u of undo.reverse()) u()
  undo = []
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
