import * as React from "react"
import { cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { useSlidingIndicator } from "./use-sliding-indicator"

afterEach(cleanup)

/** happy-dom lays nothing out: each item says where it is through `data-x`/`data-w`. */
function laidOut(el: HTMLElement | null) {
  if (!el) return
  Object.defineProperty(el, "offsetLeft", { configurable: true, get: () => Number(el.dataset.x) })
  Object.defineProperty(el, "offsetTop", { configurable: true, get: () => 4 })
  Object.defineProperty(el, "offsetWidth", { configurable: true, get: () => Number(el.dataset.w) })
  Object.defineProperty(el, "offsetHeight", { configurable: true, get: () => 24 })
}

function Row({ active, box }: { active: string | null; box?: boolean }) {
  const { listRef, indicatorRef } = useSlidingIndicator(active, { box })
  const items = [{ id: "a", x: 0, w: 40 }, { id: "b", x: 56, w: 72 }]
  return (
    <div ref={listRef} className="relative">
      {items.map((it) => (
        <button key={it.id} ref={laidOut} data-x={it.x} data-w={it.w} data-indicator-item
          data-indicator-active={it.id === active || undefined}>{it.id}</button>
      ))}
      <span ref={indicatorRef} data-testid="indicator" />
    </div>
  )
}

// Micro-interaction audit 2026-11-14 (emil): one indicator travels to the chosen item.
describe("useSlidingIndicator", () => {
  it("sits under the chosen item and moves when the choice changes", () => {
    const { getByTestId, rerender } = render(<Row active="a" />)
    const ind = getByTestId("indicator")
    expect(ind.style.transform).toBe("translateX(0px)")
    expect(ind.style.width).toBe("40px")
    rerender(<Row active="b" />)
    expect(ind.style.transform).toBe("translateX(56px)")
    expect(ind.style.width).toBe("72px")
  })

  it("covers the item's whole box with `box`", () => {
    const { getByTestId } = render(<Row active="b" box />)
    const ind = getByTestId("indicator")
    expect(ind.style.transform).toBe("translate(56px, 4px)")
    expect(ind.style.height).toBe("24px")
  })

  it("hides when nothing is chosen", () => {
    const { getByTestId } = render(<Row active={null} />)
    expect(getByTestId("indicator").style.opacity).toBe("0")
  })
})
