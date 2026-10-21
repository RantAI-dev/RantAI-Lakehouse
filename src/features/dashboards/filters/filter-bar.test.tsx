import * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import type { FilterDef, FilterField } from "@/services/clients/bi-store"
import { FilterBar } from "./filter-bar"

afterEach(cleanup)

const fields: FilterField[] = [
  { column: "province", kind: "text", tiles: 2 },
  { column: "price", kind: "number", tiles: 1 },
]
const filters: FilterDef[] = [
  { column: "province", values: ["Bali", "Aceh"] },
  { column: "price", op: "between", values: [], min: "100" },
]

function bar(over: { dirty?: boolean; canSaveDefault?: boolean; filters?: FilterDef[]; savedFilters?: FilterDef[] }) {
  render(
    <FilterBar
      board="b1"
      fields={fields}
      filters={over.filters ?? filters}
      savedFilters={over.savedFilters ?? []}
      onChange={() => {}}
      dirty={over.dirty ?? false}
      canSaveDefault={over.canSaveDefault ?? true}
      saving={false}
      onSaveDefault={() => {}}
      onReset={() => {}}
    />,
  )
}

describe("dashboard filter bar", () => {
  it("shows one chip per active filter, read in words", () => {
    bar({})
    expect(screen.getByText("province is Bali, Aceh")).toBeDefined()
    expect(screen.getByText("price ≥ 100")).toBeDefined()
  })

  it("offers neither Reset nor Save as default while the state is the default", () => {
    bar({ dirty: false })
    expect(screen.queryByText("Reset")).toBeNull()
    expect(screen.queryByText("Save as default")).toBeNull()
  })

  it("offers Reset to everyone and Save as default only to a caller who may write", () => {
    bar({ dirty: true, canSaveDefault: true })
    expect(screen.getByText("Reset")).toBeDefined()
    expect(screen.getByText("Save as default")).toBeDefined()
    cleanup()
    bar({ dirty: true, canSaveDefault: false })
    expect(screen.getByText("Reset")).toBeDefined()
    expect(screen.queryByText("Save as default")).toBeNull()
  })

  it("has no remove control on a column the saved default requires, and keeps it on the others", () => {
    bar({ savedFilters: [{ column: "province", values: ["Bali"], required: true }] })
    expect(screen.queryByLabelText("Remove province filter")).toBeNull()
    expect(screen.getByLabelText("Remove price filter")).toBeDefined()
  })

  it("lets a column go once the saved default no longer requires it", () => {
    // The state's own flag does not count until it is saved.
    bar({ filters: [{ column: "province", values: ["Bali"], required: true }], savedFilters: [{ column: "province", values: ["Bali"] }] })
    expect(screen.getByLabelText("Remove province filter")).toBeDefined()
  })

  it("shows no chip for a filter that filters nothing and still offers its column", () => {
    bar({ filters: [{ column: "province", values: [] }, { column: "price", values: [] }] })
    expect(screen.queryByLabelText("Remove province filter")).toBeNull()
    expect(screen.queryByLabelText("Remove price filter")).toBeNull()
    fireEvent.click(screen.getByText("Add filter"))
    expect(screen.getByText("province")).toBeDefined()
    expect(screen.getByText("price")).toBeDefined()
  })
})

describe("the Required switch while adding a filter", () => {
  function Harness({ canSaveDefault }: { canSaveDefault: boolean }) {
    const [state, setState] = React.useState<FilterDef[]>([])
    const [saved, setSaved] = React.useState<FilterDef[]>([])
    return (
      <FilterBar
        board="b1"
        fields={fields}
        filters={state}
        savedFilters={saved}
        onChange={setState}
        dirty={JSON.stringify(state) !== JSON.stringify(saved)}
        canSaveDefault={canSaveDefault}
        saving={false}
        onSaveDefault={() => setSaved(state)}
        onReset={() => setState(saved)}
      />
    )
  }
  const openPrice = () => {
    fireEvent.click(screen.getByText("Add filter"))
    fireEvent.click(screen.getByText("price"))
  }

  it("is offered to a caller who can save the default, with its help reachable by label", () => {
    render(<Harness canSaveDefault />)
    openPrice()
    expect(screen.getByLabelText("Require the price filter")).toBeDefined()
    expect(
      screen.getByLabelText("Viewers can change this filter's value but cannot remove it. Takes effect after Save as default."),
    ).toBeDefined()
  })

  it("is not offered to a caller who cannot", () => {
    render(<Harness canSaveDefault={false} />)
    openPrice()
    expect(screen.queryByLabelText("Require the price filter")).toBeNull()
  })

  it("creates the filter as required, keeps it removable until saved, and locks it once saved", () => {
    render(<Harness canSaveDefault />)
    openPrice()
    fireEvent.click(screen.getByLabelText("Require the price filter"))
    fireEvent.change(screen.getByLabelText("From"), { target: { value: "5" } })
    fireEvent.click(screen.getByText("Apply"))
    expect(screen.getByText("price ≥ 5")).toBeDefined()
    // Flagged in the working state but not yet the default's: still removable.
    expect(screen.getByLabelText("Remove price filter")).toBeDefined()
    fireEvent.click(screen.getByText("Save as default"))
    expect(screen.queryByLabelText("Remove price filter")).toBeNull()
  })
})

describe("the Applies to control (BI-18 part C)", () => {
  const tabs = [{ id: "t1", title: "Overview" }, { id: "t2", title: "Maps" }]
  function Harness({ canSaveDefault, withTabs = tabs, initial = [] }: { canSaveDefault: boolean; withTabs?: typeof tabs; initial?: FilterDef[] }) {
    const [state, setState] = React.useState<FilterDef[]>(initial)
    return (
      <>
        <FilterBar board="b1" fields={fields} filters={state} savedFilters={[]} onChange={setState} dirty={false}
          canSaveDefault={canSaveDefault} saving={false} onSaveDefault={() => {}} onReset={() => {}} tabs={withTabs} />
        <output data-testid="state">{JSON.stringify(state)}</output>
      </>
    )
  }
  const state = () => JSON.parse(screen.getByTestId("state").textContent ?? "[]") as FilterDef[]

  it("is offered to an editor of a dashboard with several tabs, and not otherwise", () => {
    render(<Harness canSaveDefault initial={[{ column: "province", values: ["Bali"] }]} />)
    fireEvent.click(screen.getByText("province is Bali"))
    expect(screen.getByText("Applies to")).toBeDefined()
    cleanup()
    render(<Harness canSaveDefault={false} initial={[{ column: "province", values: ["Bali"] }]} />)
    fireEvent.click(screen.getByText("province is Bali"))
    expect(screen.queryByText("Applies to")).toBeNull()
    cleanup()
    render(<Harness canSaveDefault withTabs={[tabs[0]]} initial={[{ column: "province", values: ["Bali"] }]} />)
    fireEvent.click(screen.getByText("province is Bali"))
    expect(screen.queryByText("Applies to")).toBeNull()
  })

  it("ties a filter to a tab and back to every tab", () => {
    render(<Harness canSaveDefault initial={[{ column: "province", values: ["Bali"] }]} />)
    fireEvent.click(screen.getByText("province is Bali"))
    fireEvent.click(screen.getByRole("button", { name: "Maps" }))
    expect(state()[0].tabs).toEqual(["t2"])
    fireEvent.click(screen.getByRole("button", { name: "All tabs" }))
    expect(state()[0].tabs).toBeUndefined()
  })
})
