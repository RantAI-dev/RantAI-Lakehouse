import * as React from "react"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"

// Two sets: the composer's reserve (owner QA 2026-11-14) runs against a stubbed chat state, and the
// open and close (owner QA 2026-11-15) against the real `CopilotProvider`. `fakeCopilot` picks one;
// the address is stubbed for both, and the real modules go back afterwards (`mock.module` outlives
// a file in `bun test src`).
import * as realCopilot from "./use-copilot"
import * as realNavigation from "next/navigation"

const realCopilotModule = { ...realCopilot }
const realNavigationModule = { ...realNavigation }
let fakeCopilot = true
let dockPosition: "bottom" | "right" = "bottom"
let pathname = "/dashboards/b1/charts/new"
mock.module("./use-copilot", () => ({
  ...realCopilotModule,
  useCopilot: () => {
    const real = realCopilotModule.useCopilot()
    if (!fakeCopilot) return real
    return {
      dockPosition, expanded: false, mode: "ask", setMode: () => {}, send: () => {}, setExpanded: () => {}, stop: () => {},
      busy: false, enabledCaps: [], toggleCap: () => {}, messages: [], pageContext: null,
    }
  },
}))
mock.module("next/navigation", () => ({
  ...realNavigationModule,
  usePathname: () => pathname,
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({ push: () => {}, replace: () => {}, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }),
}))
afterAll(() => {
  mock.module("./use-copilot", () => realCopilotModule)
  mock.module("next/navigation", () => realNavigationModule)
})

import { SidePanelProvider } from "@/components/app-shell/side-panel"
import { CopilotDock, dockReserve } from "./copilot-dock"
import { CopilotProvider } from "./use-copilot"

const reserve = () => document.documentElement.style.getPropertyValue("--dock-reserve")
let fetchSpy: ReturnType<typeof spyOn> | null = null

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

// Owner QA 2026-11-15: the floating dock's open and close. The motion itself is checked by eye;
// these pin what the motion must not break: where focus goes, Escape, and the draft surviving the
// swap between the pill and the open box.
function mount(expanded = false) {
  window.localStorage.setItem("copilot-dock-pos", "bottom")
  window.localStorage.setItem("copilot-dock-exp", expanded ? "1" : "0")
  fetchSpy = spyOn(globalThis, "fetch").mockImplementation((() => Promise.resolve(new Response(JSON.stringify({ sessions: [] }), { status: 200 }))) as unknown as typeof fetch)
  render(
    <CopilotProvider>
      <SidePanelProvider>
        <CopilotDock />
      </SidePanelProvider>
    </CopilotProvider>,
  )
}

const pill = () => document.querySelector<HTMLInputElement>("[data-dock-pill] input")
const box = () => document.querySelector<HTMLTextAreaElement>("textarea")
// Compared as a name: a failed `toBe` on two DOM nodes makes the runner print both trees.
const describeFocus = () => (document.activeElement === pill() ? "pill" : document.activeElement?.outerHTML.slice(0, 120))

describe("the floating AI Copilot dock (owner QA 2026-11-15)", () => {
  beforeEach(() => { fakeCopilot = false; pathname = "/dashboards/b_1" })
  afterEach(() => {
    fakeCopilot = true
    fetchSpy?.mockRestore()
    fetchSpy = null
    window.localStorage.clear()
  })

  it("opens from the pill with the focus in the message box", async () => {
    mount()
    const input = await waitFor(() => { const el = pill(); if (!el) throw new Error("no pill"); return el })
    fireEvent.focus(input)
    await waitFor(() => expect(box() !== null).toBe(true))
    expect(document.activeElement === box()).toBe(true)
    expect(screen.getByRole("button", { name: "Collapse panel" })).toBeTruthy()
  })

  it("folds back into the pill on Escape, gives the pill the focus, and does not open again from that focus", async () => {
    mount()
    fireEvent.focus(await waitFor(() => { const el = pill(); if (!el) throw new Error("no pill"); return el }))
    const textarea = await waitFor(() => { const el = box(); if (!el) throw new Error("no box"); return el })
    fireEvent.keyDown(textarea, { key: "Escape" })
    // The open stack stays for its exit, then leaves.
    await waitFor(() => expect(box() === null).toBe(true))
    expect(describeFocus()).toBe("pill")
    expect(screen.queryByRole("button", { name: "Collapse panel" })).toBeNull()
  })

  it("keeps a draft typed in the open box through a close and the next open", async () => {
    mount()
    fireEvent.focus(await waitFor(() => { const el = pill(); if (!el) throw new Error("no pill"); return el }))
    const textarea = await waitFor(() => { const el = box(); if (!el) throw new Error("no box"); return el })
    fireEvent.change(textarea, { target: { value: "rows per day" } })
    fireEvent.click(screen.getByRole("button", { name: "Collapse panel" }))
    await waitFor(() => expect(box() === null).toBe(true))
    expect(pill()?.value).toBe("rows per day")
    fireEvent.click(pill() as HTMLInputElement)
    await waitFor(() => expect(box()?.value).toBe("rows per day"))
  })

  it("keeps the page's reserve while the open box takes over from the pill, and drops it only when the dock goes", async () => {
    mount()
    fireEvent.focus(await waitFor(() => { const el = pill(); if (!el) throw new Error("no pill"); return el }))
    await waitFor(() => expect(box() !== null).toBe(true))
    // The pill leaves after its exit; the box's composer still holds the reserve.
    await waitFor(() => expect(pill() === null).toBe(true))
    expect(reserve() !== "").toBe(true)
    cleanup()
    expect(reserve()).toBe("")
  })

  it("comes back open from the last visit without taking the focus", async () => {
    mount(true)
    await waitFor(() => expect(box() !== null).toBe(true))
    expect(document.activeElement === box()).toBe(false)
  })

  it("leaves an Escape pressed in a menu opened from the dock to that menu", async () => {
    mount()
    fireEvent.focus(await waitFor(() => { const el = pill(); if (!el) throw new Error("no pill"); return el }))
    await waitFor(() => expect(box() !== null).toBe(true))
    fireEvent.click(screen.getByRole("button", { name: "Chat history" }))
    const menu = await screen.findByRole("menu")
    fireEvent.keyDown(menu, { key: "Escape" })
    await new Promise((r) => setTimeout(r, 50))
    expect(box() !== null).toBe(true)
  })

  it("leaves an Escape a popover inside already handled to that popover", async () => {
    mount()
    fireEvent.focus(await waitFor(() => { const el = pill(); if (!el) throw new Error("no pill"); return el }))
    const textarea = await waitFor(() => { const el = box(); if (!el) throw new Error("no box"); return el })
    const handled = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })
    handled.preventDefault()
    textarea.dispatchEvent(handled)
    await new Promise((r) => setTimeout(r, 50))
    expect(box() !== null).toBe(true)
  })
})
