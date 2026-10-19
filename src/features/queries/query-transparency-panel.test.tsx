// DATA-12 part 3: Query Studio's estimate panel marks the tables a ClickHouse
// query reads, and warns about a deprecated one without blocking anything.
import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import type { ActionState } from "@/hooks/use-service"
import type { QueryEstimate } from "@/services/contracts/queries"
import { QueryTransparencyPanel } from "./query-transparency-panel"

afterEach(cleanup)

const BASE: QueryEstimate = {
  estimatedBytes: 640,
  estimatedCostMin: 0.0003,
  estimatedCostMax: 0.0006,
  workloadClass: "hot-analytics",
  engine: "hot-store",
  cacheEligible: null,
  freshnessLagSeconds: null,
  policyObligations: null,
  sources: ["silver.orders", "serving.stock"],
  plan: null,
}

function panel(data: QueryEstimate) {
  const state: ActionState<QueryEstimate> = { status: "success", data, error: null }
  return render(<QueryTransparencyPanel state={state} estimatable onRetry={() => {}} />)
}

describe("QueryTransparencyPanel sources", () => {
  it("marks a certified source and says nothing about unmarked ones", () => {
    panel({ ...BASE, sourceMarks: { "serving.stock": { certification: "certified" } } })
    expect(screen.getByText("Certified")).toBeTruthy()
    expect(screen.queryByText("Deprecated")).toBeNull()
    expect(screen.queryByRole("note")).toBeNull()
  })

  it("warns about a deprecated source with its note and a link to the replacement", () => {
    panel({
      ...BASE,
      sourceMarks: {
        "silver.orders": {
          certification: "deprecated",
          certificationNote: "Superseded by the v2 table",
          replacementAssetId: "silver.orders_v2",
          replacementName: "Orders v2",
        },
      },
    })
    expect(screen.getByText("Deprecated")).toBeTruthy()
    const note = screen.getByRole("note")
    expect(note.textContent).toContain("Superseded by the v2 table.")
    expect(screen.getByRole("link", { name: "Orders v2" }).getAttribute("href")).toBe(
      "/data/assets/silver.orders_v2"
    )
  })

  it("shows the plain list when the estimate has no marks", () => {
    panel(BASE)
    expect(screen.getByText("silver.orders")).toBeTruthy()
    expect(screen.queryByText("Certified")).toBeNull()
    expect(screen.queryByText("Deprecated")).toBeNull()
  })

  it("renders governed and free tags for a source that carries them (PR #106 review R1 BLOCKER 2)", () => {
    panel({
      ...BASE,
      sourceMarks: {
        "silver.orders": {
          tags: ["pii"],
          governedTags: { data_domain: "finance" },
        },
      },
    })
    expect(screen.getByText("data_domain: finance")).toBeTruthy()
    expect(screen.getByText("pii")).toBeTruthy()
  })
})
