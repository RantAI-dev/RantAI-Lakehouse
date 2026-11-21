import { act, cleanup, render } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import { LoginContours } from "./login-contours"

const realObserver = globalThis.IntersectionObserver

/** The observers the contours create, so a test can say when the panel is on screen. */
let observers: ((entries: { isIntersecting: boolean }[]) => void)[] = []
class FakeObserver {
  constructor(cb: (entries: { isIntersecting: boolean }[]) => void) {
    observers.push(cb)
  }
  observe() {}
  disconnect() {}
  unobserve() {}
  takeRecords() {
    return []
  }
}

function setVisibility(state: "visible" | "hidden"): void {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state })
  document.dispatchEvent(new Event("visibilitychange"))
}

beforeEach(() => {
  observers = []
  globalThis.IntersectionObserver = FakeObserver as unknown as typeof IntersectionObserver
})

afterEach(() => {
  cleanup()
  globalThis.IntersectionObserver = realObserver
  setVisibility("visible")
})

describe("the lake contours behind the sign-in preview (WEB-15 T1)", () => {
  it("draws the three water levels as hidden decoration, standing still until the panel is seen", () => {
    const { container } = render(<LoginContours />)
    const root = container.querySelector(".login-contours")
    expect(root?.getAttribute("aria-hidden")).toBe("true")
    expect([...container.querySelectorAll(".login-contour")].map((el) => el.getAttribute("data-phase"))).toEqual(["0", "1", "2"])
    // Not measured yet (and before the page's script runs): the CSS keeps the drift paused.
    expect(root?.hasAttribute("data-playing")).toBe(false)
  })

  it("lets the water move only while the panel is on screen and the tab is in front", () => {
    const { container } = render(<LoginContours />)
    const playing = () => container.querySelector(".login-contours")?.hasAttribute("data-playing")

    act(() => observers.forEach((cb) => cb([{ isIntersecting: true }])))
    expect(playing()).toBe(true)

    act(() => setVisibility("hidden"))
    expect(playing()).toBe(false)
    act(() => setVisibility("visible"))
    expect(playing()).toBe(true)

    act(() => observers.forEach((cb) => cb([{ isIntersecting: false }])))
    expect(playing()).toBe(false)
  })
})
