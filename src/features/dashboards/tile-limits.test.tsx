import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { Rows } from "./tile-dialogs"
import { ReportingProvider } from "./reporting-context"

// BI-5 (feature decision 5): what a tile says when a limit ended or cut it. The chart needs a canvas; the sentence does not.
mock.module("./echart", () => ({ EChart: () => <div data-testid="chart" /> }))
const { TileBody } = await import("./tile-body")
afterEach(cleanup)

const spec = { id: "c_1", title: "Visits", kind: "bar", mart: "m", x: "p", y: "v", source: "ui" } as unknown as ChartRenderSpec
const show = (cell: Rows | { error: string; errorId?: string; limit?: unknown }) =>
  render(
    <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday" }}>
      <TileBody spec={spec} cell={cell as never} dark={false} loading={false} />
    </ReportingProvider>,
  )

describe("a tile and the query limits", () => {
  it("says the time limit and whose it was, instead of the engine's error", () => {
    show({ error: "database error", errorId: "ref-1", limit: { kind: "time", value: 30, source: { kind: "role", name: "Analyst" } } })
    expect(screen.getByText("This chart stopped after 30 seconds, the limit for your role.")).toBeTruthy()
    expect(screen.queryByText(/database error/)).toBeNull()
  })

  it("says the install's limit when no role set one", () => {
    show({ error: "x", limit: { kind: "time", value: 60, source: { kind: "install" } } })
    expect(screen.getByText("This chart stopped after 60 seconds, the limit for this install.")).toBeTruthy()
  })

  it("keeps the error text for a failure that is not a limit", () => {
    show({ error: "database error", errorId: "ref-2" })
    expect(screen.getByText(/database error/)).toBeTruthy()
  })

  it("says the row limit under a tile it cut, and draws the tile", () => {
    show({ columns: ["p", "v"], rows: [{ p: "a", v: 1 }], limitHit: { kind: "rows", value: 2000, source: { kind: "install" } } })
    expect(screen.getByTestId("chart")).toBeTruthy()
    expect(screen.getByText("Showing the first 2,000 rows, the limit set by your administrator.")).toBeTruthy()
  })

  it("says nothing about limits for a tile that no limit touched", () => {
    show({ columns: ["p", "v"], rows: [{ p: "a", v: 1 }] })
    expect(screen.queryByText(/limit/)).toBeNull()
  })
})
