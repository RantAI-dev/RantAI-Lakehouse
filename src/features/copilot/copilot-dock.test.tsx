// Owner QA 2026-11-15: the floating dock's open and close. The motion itself is
// checked by eye; these pin what the motion must not break: where focus goes,
// Escape, and the draft surviving the swap between the pill and the open box.
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/b_1",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({ push: () => {}, replace: () => {}, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }),
}))

import * as React from "react"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { SidePanelProvider } from "@/components/app-shell/side-panel"
import { CopilotDock } from "./copilot-dock"
import { CopilotProvider } from "./use-copilot"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

function mount(expanded = false) {
  window.localStorage.setItem("copilot-dock-pos", "bottom")
  window.localStorage.setItem("copilot-dock-exp", expanded ? "1" : "0")
  spyOn(globalThis, "fetch").mockImplementation((() => Promise.resolve(new Response(JSON.stringify({ sessions: [] }), { status: 200 }))) as unknown as typeof fetch)
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

describe("the floating AI Copilot dock", () => {
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
