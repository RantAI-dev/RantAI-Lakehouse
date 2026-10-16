import { cleanup, render, screen } from "@testing-library/react"
import type { AccessorFnColumnDef } from "@tanstack/react-table"
import { afterEach, describe, expect, it } from "bun:test"
import type { Pipeline } from "@/services/contracts/pipelines"
import { getPipelineColumns } from "./pipeline-columns"

afterEach(cleanup)

function makePipeline(overrides: Partial<Pipeline> = {}): Pipeline {
  return {
    id: "pl-test",
    name: "Test Pipeline",
    kind: "batch",
    status: "ready",
    owner: "system",
    source: "bronze.orders",
    target: "silver.orders_clean",
    schedule: "manual",
    lastRunAt: null,
    slaOk: null,
    freshnessLagSeconds: null,
    targetLag: { kind: "lag", minutes: 10 },
    lag: {
      status: "met",
      lagSeconds: 45,
      lastCheckAt: "2026-10-15T12:00:00Z",
      reason: null,
    },
    ...overrides,
  }
}

describe("pipeline-columns lag column", () => {
  const columns = getPipelineColumns()
  // `ColumnDef` is a union, so `.accessorFn` is not readable off it, and
  // `.find` returns `undefined` when no column matches. The lag column sets
  // both `id` and `accessorFn`, so it is an `AccessorFnColumnDef`: saying so
  // once here keeps the assertions below about what the column produces.
  const lagColumn = columns.find((c) => c.id === "lag") as AccessorFnColumnDef<Pipeline>

  // The cell template takes the table's full `CellContext`; the tests only
  // care about `row.original`, the same narrowing the Data Explorer column
  // test makes (`src/features/catalog/data-explorer-columns.test.tsx:20`).
  const lagCell = (pipeline: Pipeline): React.ReactNode =>
    (lagColumn.cell as (ctx: { row: { original: Pipeline } }) => React.ReactNode)({
      row: { original: pipeline },
    })

  it("defines the lag column with correct id and meta", () => {
    expect(lagColumn).toBeDefined()
    expect(lagColumn.id).toBe("lag")
    expect(lagColumn.meta?.label).toBe("Lag")
  })

  it("accessor extracts lag status when targetLag is present", () => {
    const p1 = makePipeline({ lag: { status: "met", lagSeconds: 10, lastCheckAt: null, reason: null } })
    const p2 = makePipeline({ targetLag: null, lag: null })

    // `AccessorFn` takes the row and its index; the lag accessor reads only
    // the row.
    expect(lagColumn.accessorFn(p1, 0)).toBe("met")
    expect(lagColumn.accessorFn(p2, 0)).toBe("")
  })

  it("renders Met pill when status is met", () => {
    const p = makePipeline({ lag: { status: "met", lagSeconds: 10, lastCheckAt: null, reason: null } })
    render(<>{lagCell(p)}</>)
    expect(screen.getByText("Met")).toBeTruthy()
  })

  it("renders Behind pill when status is behind", () => {
    const p = makePipeline({ lag: { status: "behind", lagSeconds: 700, lastCheckAt: null, reason: "slow" } })
    render(<>{lagCell(p)}</>)
    expect(screen.getByText("Behind")).toBeTruthy()
  })

  it("renders Not measured pill when status is not_measured", () => {
    const p = makePipeline({ lag: { status: "not_measured", lagSeconds: null, lastCheckAt: null, reason: null } })
    render(<>{lagCell(p)}</>)
    expect(screen.getByText("Not measured")).toBeTruthy()
  })

  it("renders dash when targetLag is null", () => {
    const p = makePipeline({ targetLag: null, lag: null })
    render(<>{lagCell(p)}</>)
    expect(screen.getByText("—")).toBeTruthy()
  })

  it("renders Not measured when targetLag is set but lag is null", () => {
    // BLD-2 review R1 SHOULD-FIX 19
    const p = makePipeline({ targetLag: { kind: "lag", minutes: 10 }, lag: null })
    render(<>{lagCell(p)}</>)
    expect(screen.getByText("Not measured")).toBeTruthy()
  })
})
