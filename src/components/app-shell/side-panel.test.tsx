// BI-61 T6 (spec T4, AC4): one side panel at a time. A page's Comments and
// Details are tabs of the panel the AI Copilot docks in: opening one replaces
// what the panel shows, and the Copilot is a tab away.
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/b_1",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({ push: () => {}, replace: () => {}, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }),
}))

import * as React from "react"
import { createPortal } from "react-dom"
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { CopilotDock } from "@/features/copilot/copilot-dock"
import { CopilotSidebar } from "@/features/copilot/copilot-sidebar"
import { CopilotProvider, useCopilot } from "@/features/copilot/use-copilot"
import { SidePanelProvider, useSidePanel, useSidePanelTabs } from "./side-panel"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

const TABS = [{ id: "comments", label: "Comments" }, { id: "details", label: "Details" }]

/** A page like the dashboard: it registers two tabs and draws the open one into the panel. */
function Page() {
  useSidePanelTabs(TABS)
  const panel = useSidePanel()
  const copilot = useCopilot()
  return (
    <div>
      <button type="button" onClick={() => panel.open("comments")}>open comments</button>
      <button type="button" onClick={() => panel.toggle("comments")}>comments button</button>
      <button type="button" onClick={() => copilot.setExpanded(true)}>ask the copilot</button>
      {panel.host && panel.active ? createPortal(<p>{panel.active} body</p>, panel.host) : null}
    </div>
  )
}

function mount({ dock, expanded, assistant = true }: { dock: "right" | "bottom"; expanded: boolean; assistant?: boolean }) {
  window.localStorage.setItem("copilot-dock-pos", dock)
  window.localStorage.setItem("copilot-dock-exp", expanded ? "1" : "0")
  spyOn(globalThis, "fetch").mockImplementation((() => Promise.resolve(new Response(JSON.stringify({ sessions: [] }), { status: 200 }))) as unknown as typeof fetch)
  const shell = (
    <SidePanelProvider>
      <Page />
      <CopilotSidebar hasAssistant={assistant} />
      {assistant ? <CopilotDock /> : null}
    </SidePanelProvider>
  )
  render(assistant ? <CopilotProvider>{shell}</CopilotProvider> : shell)
}

const panels = () => document.querySelectorAll("[data-slot='copilot-sidebar']")

describe("the shell's one side panel (BI-61 T6)", () => {
  it("opening Comments while the Copilot is docked on the right swaps the panel's content; it never adds a second panel", async () => {
    mount({ dock: "right", expanded: true })
    await waitFor(() => expect(panels()).toHaveLength(1))
    expect(screen.getByRole("tab", { name: "AI Copilot" }).getAttribute("aria-selected")).toBe("true")
    fireEvent.click(screen.getByText("open comments"))
    await screen.findByText("comments body")
    expect(panels()).toHaveLength(1)
    expect(screen.getByRole("tab", { name: "Comments" }).getAttribute("aria-selected")).toBe("true")
    // The Copilot is a tab away, in the same panel.
    fireEvent.click(screen.getByRole("tab", { name: "AI Copilot" }))
    await waitFor(() => expect(screen.queryByText("comments body")).toBeNull())
    expect(panels()).toHaveLength(1)
    expect(screen.getByRole("tab", { name: "AI Copilot" }).getAttribute("aria-selected")).toBe("true")
  })

  it("switches between the page's tabs in place", async () => {
    mount({ dock: "right", expanded: false })
    fireEvent.click(screen.getByText("open comments"))
    await screen.findByText("comments body")
    fireEvent.click(screen.getByRole("tab", { name: "Details" }))
    expect(await screen.findByText("details body")).toBeTruthy()
    expect(panels()).toHaveLength(1)
  })

  it("the Copilot opening replaces a page tab", async () => {
    mount({ dock: "right", expanded: false })
    fireEvent.click(screen.getByText("open comments"))
    await screen.findByText("comments body")
    fireEvent.click(screen.getByText("ask the copilot"))
    await waitFor(() => expect(screen.queryByText("comments body")).toBeNull())
    expect(panels()).toHaveLength(1)
    expect(screen.getByRole("tab", { name: "AI Copilot" }).getAttribute("aria-selected")).toBe("true")
  })

  it("with the Copilot docked at the bottom, its rising chat folds away and its bar steps aside while a page tab is open", async () => {
    mount({ dock: "bottom", expanded: true })
    await waitFor(() => expect(screen.getByRole("button", { name: "Collapse panel" })).toBeTruthy())
    fireEvent.click(screen.getByText("open comments"))
    await screen.findByText("comments body")
    expect(screen.queryByRole("button", { name: "Collapse panel" })).toBeNull()
    expect(screen.queryByPlaceholderText("Ask anything about your lakehouse…")).toBeNull()
    // Close (owner QA 2026-11-15, no close button any more): the dock button
    // stays in the header on a page tab; the panel goes, the bar comes back.
    expect(screen.queryByRole("button", { name: "Close panel" })).toBeNull()
    act(() => { fireEvent.click(screen.getByRole("button", { name: "Switch to bottom floating dock" })) })
    await waitFor(() => expect(panels()).toHaveLength(0))
    expect(screen.getByPlaceholderText("Ask anything about your lakehouse…")).toBeTruthy()
  })

  it("closes the whole panel when the page's button that opened it is pressed again", async () => {
    mount({ dock: "right", expanded: true })
    fireEvent.click(screen.getByText("comments button"))
    await screen.findByText("comments body")
    fireEvent.click(screen.getByText("comments button"))
    await waitFor(() => expect(panels()).toHaveLength(0))
  })

  it("closes on Escape pressed inside the panel, a page tab's portalled content included", async () => {
    mount({ dock: "right", expanded: true })
    fireEvent.click(screen.getByText("open comments"))
    const body = await screen.findByText("comments body")
    fireEvent.keyDown(body, { key: "Escape" })
    await waitFor(() => expect(panels()).toHaveLength(0))
  })

  it("keeps the dock button last in the header on every tab, with the Copilot's tools only while it shows, and no mode badge", async () => {
    mount({ dock: "right", expanded: true })
    await waitFor(() => expect(panels()).toHaveLength(1))
    const header = () => document.querySelector("[data-slot='side-panel-header']")
    expect(header()).toBeTruthy()
    const tools = () => Array.from(header()?.querySelectorAll("button[aria-label]") ?? []).map((b) => b.getAttribute("aria-label"))
    expect(tools()).toEqual(["Chat history", "New chat", "Switch to bottom floating dock"])
    expect(header()?.textContent).not.toContain("ask")
    // On a page tab the Copilot's own tools go, the dock button stays.
    fireEvent.click(screen.getByText("open comments"))
    await screen.findByText("comments body")
    expect(tools()).toEqual(["Switch to bottom floating dock"])
  })

  it("folds history and New chat into a ⋯ menu when the row is too narrow, never cutting a tab", async () => {
    // happy-dom lays nothing out: give the header 330 px and the tabs 265 px, too little for the tabs and three tools.
    const client = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth")
    const scroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollWidth")
    Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get() { return (this as HTMLElement).dataset.slot === "side-panel-header" ? 330 : 0 } })
    Object.defineProperty(HTMLElement.prototype, "scrollWidth", { configurable: true, get() { return (this as HTMLElement).getAttribute("role") === "tablist" ? 265 : 0 } })
    try {
      mount({ dock: "right", expanded: true })
      expect(await screen.findByRole("button", { name: "More AI Copilot actions" })).toBeTruthy()
      expect(screen.queryByRole("button", { name: "New chat" })).toBeNull()
      expect(screen.getByRole("tab", { name: "Comments" })).toBeTruthy()
      expect(screen.getByRole("button", { name: "Switch to bottom floating dock" })).toBeTruthy()
    } finally {
      // Put back what was there; when the getters live further up the chain, drop the stand-ins.
      if (client) Object.defineProperty(HTMLElement.prototype, "clientWidth", client)
      else Reflect.deleteProperty(HTMLElement.prototype, "clientWidth")
      if (scroll) Object.defineProperty(HTMLElement.prototype, "scrollWidth", scroll)
      else Reflect.deleteProperty(HTMLElement.prototype, "scrollWidth")
    }
  })

  it("opens the page's tabs alone for someone without the assistant", async () => {
    mount({ dock: "bottom", expanded: false, assistant: false })
    fireEvent.click(screen.getByText("open comments"))
    await screen.findByText("comments body")
    expect(screen.queryByRole("tab", { name: "AI Copilot" })).toBeNull()
    expect(screen.getByRole("tab", { name: "Comments" })).toBeTruthy()
  })

  it("is the Copilot's sidebar as before on a page that registers no tabs", async () => {
    window.localStorage.setItem("copilot-dock-pos", "right")
    window.localStorage.setItem("copilot-dock-exp", "1")
    spyOn(globalThis, "fetch").mockImplementation((() => Promise.resolve(new Response(JSON.stringify({ sessions: [] }), { status: 200 }))) as unknown as typeof fetch)
    render(<CopilotProvider><SidePanelProvider><CopilotSidebar /></SidePanelProvider></CopilotProvider>)
    await waitFor(() => expect(panels()).toHaveLength(1))
    expect(screen.queryByRole("tablist", { name: "Side panel" })).toBeNull()
    // The navbar's AI Copilot toggle closes it; the panel header keeps only "New chat" (owner, 2026-11-14).
    expect(screen.queryByRole("button", { name: "Collapse AI Copilot" })).toBeNull()
    expect(screen.getByRole("button", { name: "New chat" })).toBeTruthy()
    // The same header as the tabbed one (owner QA 2026-11-15): a plain title, the same tools in the same order.
    const header = document.querySelector("[data-slot='side-panel-header']")
    expect(header?.querySelector("h2")?.textContent?.trim()).toBe("AI Copilot")
    expect(Array.from(header?.querySelectorAll("button[aria-label]") ?? []).map((b) => b.getAttribute("aria-label")))
      .toEqual(["Chat history", "New chat", "Switch to bottom floating dock"])
    expect(screen.queryByRole("button", { name: "Close panel" })).toBeNull()
  })
})
