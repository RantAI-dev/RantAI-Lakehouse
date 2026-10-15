import { cleanup, render, screen } from "@testing-library/react"
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
  const lagColumn = columns.find((c) => c.id === "lag")!

  it("defines the lag column with correct id and meta", () => {
    expect(lagColumn).toBeDefined()
    expect(lagColumn.id).toBe("lag")
    expect(lagColumn.meta?.label).toBe("Lag")
  })

  it("accessor extracts lag status when targetLag is present", () => {
    const p1 = makePipeline({ lag: { status: "met", lagSeconds: 10, lastCheckAt: null, reason: null } })
    const p2 = makePipeline({ targetLag: null, lag: null })

    // @ts-expect-error accessorFn test
    expect(lagColumn.accessorFn(p1)).toBe("met")
    // @ts-expect-error accessorFn test
    expect(lagColumn.accessorFn(p2)).toBe("")
  })

  it("renders Met pill when status is met", () => {
    const p = makePipeline({ lag: { status: "met", lagSeconds: 10, lastCheckAt: null, reason: null } })
    // @ts-expect-error cell test
    const Cell = lagColumn.cell!
    // @ts-expect-error cell render
    render(<Cell row={{ original: p }} />)
    expect(screen.getByText("Met")).toBeTruthy()
  })

  it("renders Behind pill when status is behind", () => {
    const p = makePipeline({ lag: { status: "behind", lagSeconds: 700, lastCheckAt: null, reason: "slow" } })
    // @ts-expect-error cell test
    const Cell = lagColumn.cell!
    // @ts-expect-error cell render
    render(<Cell row={{ original: p }} />)
    expect(screen.getByText("Behind")).toBeTruthy()
  })

  it("renders Not measured pill when status is not_measured", () => {
    const p = makePipeline({ lag: { status: "not_measured", lagSeconds: null, lastCheckAt: null, reason: null } })
    // @ts-expect-error cell test
    const Cell = lagColumn.cell!
    // @ts-expect-error cell render
    render(<Cell row={{ original: p }} />)
    expect(screen.getByText("Not measured")).toBeTruthy()
  })

  it("renders dash when targetLag is null", () => {
    const p = makePipeline({ targetLag: null, lag: null })
    // @ts-expect-error cell test
    const Cell = lagColumn.cell!
    // @ts-expect-error cell render
    render(<Cell row={{ original: p }} />)
    expect(screen.getByText("—")).toBeTruthy()
  })
})
