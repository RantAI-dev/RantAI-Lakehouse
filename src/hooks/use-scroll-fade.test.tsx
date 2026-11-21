import * as React from "react"
import { act, cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { useScrollFade } from "./use-scroll-fade"

afterEach(cleanup)

function Area() {
  const ref = useScrollFade<HTMLDivElement>()
  return <div ref={ref} data-testid="area" className="scroll-fade"><div>content</div></div>
}

/** happy-dom lays nothing out: give the area a 100 px window on 300 px of content. */
function sized(el: HTMLElement) {
  Object.defineProperty(el, "clientHeight", { configurable: true, value: 100 })
  Object.defineProperty(el, "scrollHeight", { configurable: true, value: 300 })
}

function scrollTo(el: HTMLElement, top: number) {
  act(() => {
    el.scrollTop = top
    el.dispatchEvent(new Event("scroll"))
  })
}

// BI-61 review round 1, SHOULD-FIX 1: an edge fades only while it hides something.
describe("useScrollFade", () => {
  it("has no top fade at the top, a top fade once scrolled, and no bottom fade at the very bottom", () => {
    const { getByTestId } = render(<Area />)
    const el = getByTestId("area")
    sized(el)
    scrollTo(el, 0)
    expect(el.hasAttribute("data-fade-top")).toBe(false)
    expect(el.hasAttribute("data-fade-bottom")).toBe(true)
    scrollTo(el, 80)
    expect(el.hasAttribute("data-fade-top")).toBe(true)
    expect(el.hasAttribute("data-fade-bottom")).toBe(true)
    scrollTo(el, 200)
    expect(el.hasAttribute("data-fade-top")).toBe(true)
    expect(el.hasAttribute("data-fade-bottom")).toBe(false)
  })

  it("fades no edge when everything fits", () => {
    const { getByTestId } = render(<Area />)
    const el = getByTestId("area")
    Object.defineProperty(el, "clientHeight", { configurable: true, value: 300 })
    Object.defineProperty(el, "scrollHeight", { configurable: true, value: 300 })
    scrollTo(el, 0)
    expect(el.hasAttribute("data-fade-top")).toBe(false)
    expect(el.hasAttribute("data-fade-bottom")).toBe(false)
  })
})
