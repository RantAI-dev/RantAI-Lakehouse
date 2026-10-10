import { cleanup, render, screen } from "@testing-library/react"
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

function bar(over: { dirty?: boolean; canSaveDefault?: boolean; filters?: FilterDef[] }) {
  render(
    <FilterBar
      board="b1"
      fields={fields}
      filters={over.filters ?? filters}
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
})
