let mockPermissions = ["pipeline:write"]

import { afterAll, mock } from "bun:test"

import * as realAuth from "@/features/auth/auth-provider"

// The panel asks the signed-in user whether it may save. `mock.module`
// outlives this file in `bun test src` (PR #160's note, learned the same
// way in `src/features/dashboards/chart-builder.test.tsx`), so the real
// exports are kept, only `useAuth` is replaced, and the real module goes
// back in `afterAll`: a bare `{ useAuth }` leaves every later file -- the
// catalog asset-detail tabs among them -- without `AuthProvider`.
const real = { ...realAuth }
mock.module("@/features/auth/auth-provider", () => ({
  ...real,
  useAuth: () => ({
    hasPermission: (permission: string) => mockPermissions.includes(permission),
  }),
}))
afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
})

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import { pipelineService } from "@/services"
import type { PipelineDetail } from "@/services/contracts/pipelines"
import { PipelineFreshnessPanel } from "./pipeline-freshness-panel"

afterEach(cleanup)
beforeEach(() => {
  mockPermissions = ["pipeline:write"]
})

function makePipeline(overrides: Partial<PipelineDetail> = {}): PipelineDetail {
  return {
    id: "pl-orders",
    name: "Orders Clean",
    kind: "batch",
    status: "ready",
    owner: "system",
    source: "bronze.orders",
    target: "silver.orders_clean",
    schedule: "manual",
    lastRunAt: null,
    slaOk: null,
    freshnessLagSeconds: null,
    engine: "authored",
    description: null,
    graph: null,
    config: [],
    definition: null,
    runs: [],
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

describe("PipelineFreshnessPanel display", () => {
  it("renders target lag, current lag, status pill, and last check for a lag pipeline", () => {
    const p = makePipeline()
    render(<PipelineFreshnessPanel pipeline={p} />)

    expect(screen.getByText("10 minutes")).toBeTruthy()
    expect(screen.getByText("45s")).toBeTruthy()
    expect(screen.getByText("Met")).toBeTruthy()
    expect(screen.getByLabelText("Freshness")).toBeTruthy()
  })

  it("renders Behind pill when status is behind", () => {
    const p = makePipeline({
      lag: {
        status: "behind",
        lagSeconds: 700,
        lastCheckAt: "2026-10-15T12:00:00Z",
        reason: "lag 700s exceeds target 600s",
      },
    })
    render(<PipelineFreshnessPanel pipeline={p} />)

    expect(screen.getByText("Behind")).toBeTruthy()
    expect(screen.getByText("700s")).toBeTruthy()
    expect(screen.getByText("lag 700s exceeds target 600s")).toBeTruthy()
  })

  it("renders Not measured when status is not_measured", () => {
    const p = makePipeline({
      lag: {
        status: "not_measured",
        lagSeconds: null,
        lastCheckAt: "2026-10-15T12:00:00Z",
        reason: "no check recorded in the last 90 seconds",
      },
    })
    render(<PipelineFreshnessPanel pipeline={p} />)

    expect(screen.getByText("Not measured")).toBeTruthy()
    expect(screen.getByText("no check recorded in the last 90 seconds")).toBeTruthy()
  })

  it("renders the D7 downstream note when downstream pipeline has no readers", () => {
    const p = makePipeline({
      targetLag: { kind: "downstream" },
      lag: {
        status: "not_measured",
        lagSeconds: null,
        lastCheckAt: "2026-10-15T12:00:00Z",
        reason: "Downstream: no pipeline with a target lag reads this table",
      },
    })
    render(<PipelineFreshnessPanel pipeline={p} />)

    expect(screen.getByText("Downstream")).toBeTruthy()
    expect(screen.getByText("Downstream: no pipeline with a target lag reads this table")).toBeTruthy()
  })

  it("renders Not configured when targetLag is null", () => {
    const p = makePipeline({
      targetLag: null,
      lag: null,
    })
    render(<PipelineFreshnessPanel pipeline={p} />)

    expect(screen.getByText("Not configured")).toBeTruthy()
    expect(screen.getByText("None (runs on schedule or demand)")).toBeTruthy()
  })

  // BLD-2 review R2 SHOULD-FIX 10: unknown widened kind renders honestly
  it("renders Unknown target lag when kind is unknown", () => {
    const p = makePipeline({
      targetLag: { kind: "future_kind" } as any,
    })
    render(<PipelineFreshnessPanel pipeline={p} />)

    expect(screen.getByText("Unknown target lag")).toBeTruthy()
  })
})

describe("PipelineFreshnessPanel authorization and controls", () => {
  it("hides the configure button when caller lacks pipeline:write", () => {
    mockPermissions = ["pipeline:read"]
    const p = makePipeline()
    render(<PipelineFreshnessPanel pipeline={p} />)

    expect(screen.queryByLabelText("Configure target lag")).toBeNull()
  })

  it("hides the configure button for non-authored (Dagster) jobs", () => {
    const p = makePipeline({ engine: "dagster" })
    render(<PipelineFreshnessPanel pipeline={p} />)

    expect(screen.queryByLabelText("Configure target lag")).toBeNull()
  })

  it("allows setting target lag in minutes", async () => {
    const p = makePipeline()
    let updatedPayload: unknown = null
    const orig = pipelineService.setTargetLag
    pipelineService.setTargetLag = async (id, targetLag) => {
      updatedPayload = targetLag
      return { ...p, targetLag }
    }

    try {
      let reloaded = false
      render(<PipelineFreshnessPanel pipeline={p} onUpdated={() => { reloaded = true }} />)

      const configBtn = screen.getByLabelText("Configure target lag")
      fireEvent.click(configBtn)

      const minutesInput = screen.getByLabelText("Minutes (1 - 10080)") as HTMLInputElement
      fireEvent.change(minutesInput, { target: { value: "25" } })

      const saveBtn = screen.getByText("Save target lag")
      fireEvent.click(saveBtn)

      await waitFor(() => {
        expect(updatedPayload).toEqual({ kind: "lag", minutes: 25 })
        expect(reloaded).toBe(true)
      })
    } finally {
      pipelineService.setTargetLag = orig
    }
  })

  it("allows setting target lag to downstream", async () => {
    const p = makePipeline()
    let updatedPayload: unknown = null
    const orig = pipelineService.setTargetLag
    pipelineService.setTargetLag = async (id, targetLag) => {
      updatedPayload = targetLag
      return { ...p, targetLag }
    }

    try {
      render(<PipelineFreshnessPanel pipeline={p} />)

      fireEvent.click(screen.getByLabelText("Configure target lag"))
      fireEvent.click(screen.getByText("Downstream"))
      const form = screen.getByText("Save target lag").closest("form")!
      fireEvent.submit(form)

      await waitFor(() => {
        expect(updatedPayload).toEqual({ kind: "downstream" })
      })
    } finally {
      pipelineService.setTargetLag = orig
    }
  })

  it("allows clearing target lag to null", async () => {
    const p = makePipeline()
    let updatedPayload: unknown = "not-called"
    const orig = pipelineService.setTargetLag
    pipelineService.setTargetLag = async (id, targetLag) => {
      updatedPayload = targetLag
      return { ...p, targetLag }
    }

    try {
      render(<PipelineFreshnessPanel pipeline={p} />)

      fireEvent.click(screen.getByLabelText("Configure target lag"))
      fireEvent.click(screen.getByText("None (clear)"))
      const form = screen.getByText("Save target lag").closest("form")!
      fireEvent.submit(form)

      await waitFor(() => {
        expect(updatedPayload).toBeNull()
      })
    } finally {
      pipelineService.setTargetLag = orig
    }
  })

  it("surfaces honest API error text on rejection", async () => {
    const p = makePipeline()
    const orig = pipelineService.setTargetLag
    pipelineService.setTargetLag = async () => {
      throw new Error("target lag would create a cycle: pl-orders -> pl-upstream -> pl-orders")
    }

    try {
      render(<PipelineFreshnessPanel pipeline={p} />)

      fireEvent.click(screen.getByLabelText("Configure target lag"))
      const form = screen.getByText("Save target lag").closest("form")!
      fireEvent.submit(form)

      await waitFor(() => {
        expect(
          screen.getByText("target lag would create a cycle: pl-orders -> pl-upstream -> pl-orders")
        ).toBeTruthy()
      })
    } finally {
      pipelineService.setTargetLag = orig
    }
  })

  // BLD-2 review R2 SHOULD-FIX 10: minutes below floor is refused with floor message
  it("refuses minutes below the configured floor", async () => {
    const p = makePipeline()
    let called = false
    const orig = pipelineService.setTargetLag
    pipelineService.setTargetLag = async (id, targetLag) => {
      called = true
      return { ...p, targetLag }
    }

    try {
      render(<PipelineFreshnessPanel pipeline={p} />)

      fireEvent.click(screen.getByLabelText("Configure target lag"))

      const minutesInput = screen.getByLabelText("Minutes (1 - 10080)") as HTMLInputElement
      fireEvent.change(minutesInput, { target: { value: "0" } })

      const form = screen.getByText("Save target lag").closest("form")!
      fireEvent.submit(form)

      await waitFor(() => {
        expect(screen.getByRole("alert").textContent).toContain("Minutes must be between 1 and 10080")
        expect(called).toBe(false)
      })
    } finally {
      pipelineService.setTargetLag = orig
    }
  })
})
