import * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import type { BoardTab } from "@/services"
import { TabBar, type TabEditing } from "./tab-bar"

afterEach(cleanup)

const tabs: BoardTab[] = [{ id: "t1", title: "Overview" }, { id: "t2", title: "Maps" }]

function editing(over: Partial<TabEditing> = {}): TabEditing {
  return { onAdd: () => {}, onRename: () => {}, onMove: () => {}, onDelete: () => {}, tileCount: () => 0, ...over }
}

describe("dashboard tab bar", () => {
  it("shows a reader the tabs, marks the selected one and reports a choice", () => {
    let chosen = ""
    render(<TabBar tabs={tabs} selected="t1" onSelect={(id) => { chosen = id }} />)
    expect(screen.getByRole("tab", { name: "Overview" }).getAttribute("aria-selected")).toBe("true")
    expect(screen.getByRole("tab", { name: "Maps" }).getAttribute("aria-selected")).toBe("false")
    fireEvent.click(screen.getByRole("tab", { name: "Maps" }))
    expect(chosen).toBe("t2")
    expect(screen.queryByText("Tab")).toBeNull()
  })

  it("shows nothing to a reader of a dashboard with no tabs", () => {
    const { container } = render(<TabBar tabs={[]} onSelect={() => {}} />)
    expect(container.textContent).toBe("")
  })

  it("offers an editor of a dashboard with no tabs a way to start", () => {
    render(<TabBar tabs={[]} onSelect={() => {}} editing={editing()} />)
    expect(screen.getByText("Add tabs")).toBeDefined()
  })

  it("adds a tab by name", () => {
    let added = ""
    render(<TabBar tabs={tabs} selected="t1" onSelect={() => {}} editing={editing({ onAdd: (t) => { added = t } })} />)
    fireEvent.click(screen.getByText("Tab"))
    fireEvent.change(screen.getByLabelText("Tab name"), { target: { value: "  Trends " } })
    fireEvent.click(screen.getByText("Add"))
    expect(added).toBe("Trends")
  })

  it("will not add a blank name", () => {
    render(<TabBar tabs={tabs} selected="t1" onSelect={() => {}} editing={editing()} />)
    fireEvent.click(screen.getByText("Tab"))
    expect((screen.getByText("Add") as HTMLButtonElement).disabled).toBe(true)
  })

  it("stops offering new tabs at twenty", () => {
    const many = Array.from({ length: 20 }, (_, i) => ({ id: `t${i}`, title: `T${i}` }))
    render(<TabBar tabs={many} selected="t0" onSelect={() => {}} editing={editing()} />)
    expect((screen.getByText("Tab").closest("button") as HTMLButtonElement).disabled).toBe(true)
  })
})
