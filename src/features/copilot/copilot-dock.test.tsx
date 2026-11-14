import { cleanup, render } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"

// Owner QA 2026-11-14, composer reserve: the room under the page follows the composer's visibility.
// The dock reads the chat's state and the address; both are stubbed, and the real modules go back
// afterwards (`mock.module` outlives a file in `bun test src`).
import * as realCopilot from "./use-copilot"
import * as realNavigation from "next/navigation"

const realCopilotModule = { ...realCopilot }
const realNavigationModule = { ...realNavigation }
let dockPosition: "bottom" | "right" = "bottom"
let pathname = "/dashboards/b1/charts/new"
mock.module("./use-copilot", () => ({
  ...realCopilotModule,
  useCopilot: () => ({
    dockPosition, expanded: false, mode: "ask", setMode: () => {}, send: () => {}, setExpanded: () => {}, stop: () => {},
    busy: false, enabledCaps: [], toggleCap: () => {}, messages: [], pageContext: null,
  }),
}))
mock.module("next/navigation", () => ({ ...realNavigationModule, usePathname: () => pathname }))
afterAll(() => {
  mock.module("./use-copilot", () => realCopilotModule)
  mock.module("next/navigation", () => realNavigationModule)
})

import { CopilotDock, dockReserve } from "./copilot-dock"

const reserve = () => document.documentElement.style.getPropertyValue("--dock-reserve")

beforeEach(() => {
  dockPosition = "bottom"
  pathname = "/dashboards/b1/charts/new"
})
afterEach(() => {
  cleanup()
  document.documentElement.style.removeProperty("--dock-reserve")
})

describe("the composer's reserve (owner QA 2026-11-14)", () => {
  it("is the composer's height plus its 16 px offset and a 14 px gap", () => {
    expect(dockReserve(46)).toBe(76)
    expect(dockReserve(60.4)).toBe(90)
  })

  it("is set while the composer shows, from its measured height, and removed when it hides", () => {
    const rect = spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ height: 46, width: 460, top: 0, left: 0, right: 460, bottom: 46, x: 0, y: 0, toJSON: () => ({}) } as DOMRect)
    try {
      const { unmount } = render(<CopilotDock />)
      expect(reserve()).toBe("76px")
      unmount()
      expect(reserve()).toBe("")
    } finally {
      rect.mockRestore()
    }
  })

  it("is 0 (not set) when the dock hides itself for the Copilot side panel or on a page without it", () => {
    dockPosition = "right"
    render(<CopilotDock />)
    expect(reserve()).toBe("")
    cleanup()
    dockPosition = "bottom"
    pathname = "/copilot"
    render(<CopilotDock />)
    expect(reserve()).toBe("")
  })
})
