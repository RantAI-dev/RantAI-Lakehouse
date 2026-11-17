import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { CustomFieldsMenu } from "./custom-fields-menu"
import type { CalcField } from "@/services/contracts/calc-fields"
import type { Hierarchy } from "@/services/contracts/hierarchies"

afterEach(cleanup)

const field = { id: "f1", name: "revenue_per_visit", formula: "a / b", type: "number", level: "row" } as unknown as CalcField
const group = { id: "g1", name: "island", type: "text", level: "group", formula: "{}" } as unknown as CalcField
const hierarchy = { id: "h1", name: "geography", levels: ["provinsi", "kab"] } as unknown as Hierarchy

function open(props: Partial<Parameters<typeof CustomFieldsMenu>[0]> = {}) {
  const handlers = {
    onEditField: mock(() => {}), onEditGroup: mock(() => {}), onEditHierarchy: mock(() => {}),
    onNewField: mock(() => {}), onNewGroup: mock(() => {}), onNewHierarchy: mock(() => {}),
  }
  render(<CustomFieldsMenu calculated={[field, group]} hierarchies={[hierarchy]} {...handlers} {...props} />)
  return handlers
}

describe("CustomFieldsMenu (phase 2 QA R3)", () => {
  it("counts the calculated fields, value groups and hierarchies on the button", () => {
    open()
    expect(screen.getByRole("button", { name: /Custom fields \(3\)/ })).toBeTruthy()
    // Nothing is listed until the menu is opened: no row of chips in the form.
    expect(screen.queryByText("revenue_per_visit")).toBeNull()
  })

  it("lists each with its mark and opens the right editor on a click", async () => {
    const h = open()
    fireEvent.click(screen.getByRole("button", { name: /Custom fields/ }))
    expect(await screen.findByText("revenue_per_visit")).toBeTruthy()
    expect(screen.getByText("geography")).toBeTruthy()
    for (const mark of ["fx", "group", "hierarchy"]) expect(screen.getAllByText(mark).length).toBeGreaterThan(0)
    fireEvent.click(screen.getByText("geography"))
    expect(h.onEditHierarchy).toHaveBeenCalledTimes(1)
    expect(h.onEditField).not.toHaveBeenCalled()
  })

  it("holds the three New actions even when the source has none yet", async () => {
    const h = open({ calculated: [], hierarchies: [] })
    fireEvent.click(screen.getByRole("button", { name: /Custom fields \(0\)/ }))
    fireEvent.click(await screen.findByText("New value group"))
    expect(h.onNewGroup).toHaveBeenCalledTimes(1)
    expect(screen.queryByText("On this source")).toBeNull()
  })
})
