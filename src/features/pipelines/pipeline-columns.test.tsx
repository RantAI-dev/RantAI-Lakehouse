import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import type { CellContext } from "@tanstack/react-table"
import type { Pipeline } from "@/services/contracts/pipelines"
import { getPipelineColumns, triggerBlockedReason } from "./pipeline-columns"

afterEach(cleanup)

const PIPELINE: Pipeline = {
  id: "pl-orders-clean-abc",
  name: "orders_clean",
  kind: "batch",
  status: "ready",
  owner: "Current user",
  source: "bronze.orders",
  target: "silver.orders_clean",
  schedule: "manual",
  lastRunAt: null,
  slaOk: null,
  freshnessLagSeconds: null,
}

function renderCell(columnId: string, pipeline: Pipeline) {
  const column = getPipelineColumns().find((c) => c.id === columnId)
  const cell = column?.cell
  if (typeof cell !== "function") throw new Error(`column ${columnId} has no cell renderer`)
  render(<>{cell({ row: { original: pipeline } } as CellContext<Pipeline, unknown>)}</>)
}

describe("SLA column", () => {
  it("shows no verdict when no SLA is defined", () => {
    renderCell("slaOk", PIPELINE)
    expect(screen.getByText("—")).toBeTruthy()
    expect(screen.queryByText("Breached")).toBeNull()
  })

  it("shows a real verdict when there is one", () => {
    renderCell("slaOk", { ...PIPELINE, slaOk: false })
    expect(screen.getByText("Breached")).toBeTruthy()
    cleanup()
    renderCell("slaOk", { ...PIPELINE, slaOk: true })
    expect(screen.getByText("OK")).toBeTruthy()
  })
})

describe("triggerBlockedReason", () => {
  it("lets an orchestrator job and an active authored pipeline run", () => {
    expect(triggerBlockedReason({ ...PIPELINE, id: "gold_export_job", status: "unknown" })).toBeNull()
    expect(triggerBlockedReason(PIPELINE)).toBeNull()
  })

  it("says why an authored pipeline cannot run", () => {
    expect(triggerBlockedReason({ ...PIPELINE, status: "draft" })).toBe("activate it first")
    expect(triggerBlockedReason({ ...PIPELINE, status: "paused" })).toBe("paused")
    expect(triggerBlockedReason({ ...PIPELINE, status: "archived" })).toBe("not active")
  })
})
