// UX-9: the status bar under the editor is the one place the engine and the
// pre-run estimate are written (`studio-status-bar.tsx`); the Plan tab holds
// the details and never repeats them. These are the bar's own state branches,
// read without the page around it — the page tests cover its wiring.
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ActionState } from "@/hooks/use-service"
import type { QueryEstimate } from "@/services/contracts/queries"
import { StudioStatusBar } from "./studio-status-bar"
import type { useQueryStudio } from "./use-query-studio"

afterEach(cleanup)

type Studio = ReturnType<typeof useQueryStudio>

/** A ready ClickHouse estimate, as `GET /api/query/estimate` answers it. */
const ESTIMATE: QueryEstimate = {
  supported: true,
  queryEngine: "clickhouse",
  estimatedBytes: 1024,
  estimatedCostMin: 0.01,
  estimatedCostMax: 0.02,
  error: null,
  workloadClass: "hot-analytics",
  engine: "hot-store",
  cacheEligible: null,
  freshnessLagSeconds: null,
  policyObligations: [],
  sources: ["serving.orders"],
  plan: null,
}

/** The studio fields the bar reads, over a ready ClickHouse estimate. */
function studio(over: Record<string, unknown> = {}): Studio {
  return {
    engine: "clickhouse",
    setEngine: () => {},
    estimateSupported: true,
    estimatable: true,
    estimateWaitsFor: null,
    estimateAct: { status: "success", data: ESTIMATE, error: null } as ActionState<QueryEstimate>,
    reEstimate: () => {},
    ...over,
  } as unknown as Studio
}

function bar(studioValue: Studio, onOpenPlan: () => void = () => {}) {
  return render(<StudioStatusBar studio={studioValue} onOpenPlan={onOpenPlan} />)
}

describe("StudioStatusBar estimate states", () => {
  it("names the engine and the ready estimate in one line, and opens the plan from it", async () => {
    const onOpenPlan = mock(() => {})
    bar(studio(), onOpenPlan)

    expect(await screen.findByRole("button", { name: "Engine: ClickHouse" })).toBeTruthy()
    const estimate = await screen.findByRole("button", { name: /^Estimate ≈ 1\.0 KB · / })
    expect(estimate.textContent).toContain("≈ 1.0 KB")
    expect(estimate.textContent).toContain("0.0100")
    fireEvent.click(estimate)
    expect(onOpenPlan).toHaveBeenCalledTimes(1)
  })

  it("adds cache and data age only when they were measured", async () => {
    bar(studio({ estimateAct: {
      status: "success",
      data: { ...ESTIMATE, cacheEligible: true, freshnessLagSeconds: 3 * 3600 },
      error: null,
    } as ActionState<QueryEstimate> }))

    const estimate = await screen.findByRole("button", { name: /^Estimate ≈ 1\.0 KB · / })
    expect(estimate.textContent).toContain("cache eligible")
    expect(estimate.textContent).toContain("data 3h 00m old")
  })

  it("says a Trino query has no estimate, before anything else", () => {
    bar(studio({ engine: "trino", estimateSupported: false }))
    expect(screen.getByText("No pre-run estimate for Trino")).toBeTruthy()
  })

  it("names the parameter the estimate waits for", () => {
    bar(studio({ estimatable: false, estimateWaitsFor: "region" }))
    expect(screen.getByText("Fill in {{region}} for an estimate")).toBeTruthy()
  })

  it("asks for SQL when there is nothing to estimate yet", () => {
    bar(studio({ estimatable: false, estimateAct: { status: "idle", data: null, error: null } }))
    expect(screen.getByText("Type SQL for an estimate")).toBeTruthy()
  })

  it("says the estimate is unavailable and retries from the line", () => {
    const reEstimate = mock(() => {})
    bar(studio({ estimateAct: { status: "error", data: null, error: new Error("no") }, reEstimate }))

    expect(screen.getByText("Estimate unavailable")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Retry" }))
    expect(reEstimate).toHaveBeenCalledTimes(1)
  })

  it("says it is estimating, once, while the first answer is on its way", () => {
    bar(studio({ estimateAct: { status: "pending", data: null, error: null } }))
    const estimating = screen.getByRole("status", { name: "Estimating" })
    expect(estimating.textContent).toContain("Estimating…")
  })

  it("keeps the estimate on screen, quieter, while a new one is on its way", async () => {
    bar(studio({ estimateAct: { status: "pending", data: ESTIMATE, error: null } }))

    const estimate = await screen.findByRole("button", { name: /^Estimate ≈ 1\.0 KB · / })
    expect(estimate.className).toContain("opacity-60")
    expect(screen.queryByRole("status", { name: "Estimating" })).toBeNull()
  })

  it("says the server has no estimate for this query, and opens the plan for the reason", async () => {
    const onOpenPlan = mock(() => {})
    bar(
      studio({ estimateAct: {
        status: "success",
        data: { ...ESTIMATE, supported: false, reason: "No estimate for this statement." },
        error: null,
      } as ActionState<QueryEstimate> }),
      onOpenPlan
    )

    const line = await screen.findByRole("button", { name: "No pre-run estimate: open the plan" })
    expect(line.textContent).toContain("No pre-run estimate")
    fireEvent.click(line)
    expect(onOpenPlan).toHaveBeenCalledTimes(1)
  })

  it("says the query cannot be planned, and opens the plan for the reason", async () => {
    const onOpenPlan = mock(() => {})
    bar(
      studio({ estimateAct: {
        status: "success",
        data: { ...ESTIMATE, error: "This query cannot be planned. Reference: X-1" },
        error: null,
      } as ActionState<QueryEstimate> }),
      onOpenPlan
    )

    const line = await screen.findByRole("button", {
      name: "This query cannot be planned: open the plan",
    })
    expect(line.textContent).toContain("Cannot be planned")
    fireEvent.click(line)
    expect(onOpenPlan).toHaveBeenCalledTimes(1)
  })
})
