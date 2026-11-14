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
    // Close: the panel goes, the bar comes back.
    act(() => { fireEvent.click(screen.getByRole("button", { name: "Close panel" })) })
    await waitFor(() => expect(panels()).toHaveLength(0))
    expect(screen.getByPlaceholderText("Ask anything about your lakehouse…")).toBeTruthy()
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
    expect(screen.getByRole("button", { name: "Collapse AI Copilot" })).toBeTruthy()
  })
})
