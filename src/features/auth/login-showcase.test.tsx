import { act, cleanup, render } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { useRenderMode } from "@/features/dashboards/render-mode"

// The charts need a canvas; what matters here is how the preview asks for
// them: with the still render mode (no animation) or without it.
const drawn: { still: boolean }[] = []
mock.module("@/features/dashboards/echart", () => ({
  EChart: function FakeChart() {
    drawn.push({ still: useRenderMode() !== null })
    return <div data-testid="chart" />
  },
}))
mock.module("next/image", () => ({ default: () => null }))

const { LoginShowcase, LOOP_MS, VISITORS_2024 } = await import("./login-showcase")

const FINAL = VISITORS_2024.toLocaleString("en-US")

const realMatchMedia = window.matchMedia
const realObserver = globalThis.IntersectionObserver
const realSetInterval = window.setInterval
const realClearInterval = window.clearInterval

/** `prefers-reduced-motion: reduce` answers `reduced`; every other query answers false. */
function stubReducedMotion(reduced: boolean): void {
  window.matchMedia = ((query: string) => ({
    matches: reduced && query.includes("prefers-reduced-motion: reduce"),
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia
}

/** The observers the preview creates, so a test can say when it is on screen. */
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

/** The intervals the preview has running, by id, with their delay. */
let running = new Map<number, number>()

function setVisibility(state: "visible" | "hidden"): void {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state })
  document.dispatchEvent(new Event("visibilitychange"))
}

beforeEach(() => {
  observers = []
  running = new Map()
  globalThis.IntersectionObserver = FakeObserver as unknown as typeof IntersectionObserver
  window.setInterval = ((fn: () => void, ms?: number) => {
    const id = realSetInterval(fn, ms) as unknown as number
    running.set(id, ms ?? 0)
    return id
  }) as typeof window.setInterval
  window.clearInterval = ((id?: number) => {
    if (id !== undefined) running.delete(id)
    realClearInterval(id)
  }) as typeof window.clearInterval
})

afterEach(() => {
  cleanup()
  drawn.length = 0
  for (const id of running.keys()) realClearInterval(id)
  window.matchMedia = realMatchMedia
  globalThis.IntersectionObserver = realObserver
  window.setInterval = realSetInterval
  window.clearInterval = realClearInterval
  setVisibility("visible")
})

describe("the sign-in page's sample dashboard (WEB-15 T2)", () => {
  it("draws once, still, with the final numbers, and schedules nothing under reduced motion", () => {
    stubReducedMotion(true)
    const { container } = render(<LoginShowcase />)
    act(() => observers.forEach((cb) => cb([{ isIntersecting: true }])))

    expect(container.querySelector("[data-motion]")?.getAttribute("data-motion")).toBe("still")
    // Every chart drew with the print view's still mode: no ECharts animation.
    expect(drawn.length).toBeGreaterThan(0)
    expect(drawn.every((c) => c.still)).toBe(true)
    // The KPI shows its final value from the first frame; it does not count up.
    expect(container.textContent).toContain(FINAL)
    // And no redraw loop, even on screen.
    expect([...running.values()]).not.toContain(LOOP_MS)
  })

  it("shows the phone's tile with the final KPI under reduced motion", () => {
    stubReducedMotion(true)
    const { container } = render(<LoginShowcase compact />)
    expect(container.querySelector("[data-slot=showcase-kpi]")?.textContent).toBe(FINAL)
    expect(drawn.every((c) => c.still)).toBe(true)
  })

  it("says the KPI's change on one line and does not repeat the year under the number (WEB-15 T2)", () => {
    stubReducedMotion(true)
    const { container } = render(<LoginShowcase />)
    // The tile's title already says 2024; the number, then "+19.3% vs 2023".
    const tile = container.querySelector("[data-slot=showcase-kpi]")?.parentElement
    expect(tile?.textContent).toBe(`${FINAL}+19.3%vs 2023`)
  })

  it("redraws every 9 s only while it is on screen and the tab is visible", () => {
    stubReducedMotion(false)
    const { container } = render(<LoginShowcase />)
    expect(container.querySelector("[data-motion]")?.getAttribute("data-motion")).toBe("live")
    expect(drawn.some((c) => c.still)).toBe(false)
    // Not yet on screen: no loop.
    expect([...running.values()]).not.toContain(LOOP_MS)

    act(() => observers.forEach((cb) => cb([{ isIntersecting: true }])))
    expect([...running.values()]).toContain(LOOP_MS)

    // Another tab in front: the loop stops, and comes back with the tab.
    act(() => setVisibility("hidden"))
    expect([...running.values()]).not.toContain(LOOP_MS)
    act(() => setVisibility("visible"))
    expect([...running.values()]).toContain(LOOP_MS)

    // Scrolled away: stops again.
    act(() => observers.forEach((cb) => cb([{ isIntersecting: false }])))
    expect([...running.values()]).not.toContain(LOOP_MS)
  })
})
