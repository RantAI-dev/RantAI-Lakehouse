import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"

import { ChartDraftCard, chartDefFromArgs, previewSpecWithDef } from "./chart-draft-card"

const originalFetch = global.fetch
afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

// What the server's preview answers: the render spec (no `def`) and one row.
function previewOf(spec: Record<string, unknown>, rows: Record<string, unknown>[]) {
  global.fetch = mock(async () =>
    new Response(JSON.stringify({ spec, result: { columns: Object.keys(rows[0] ?? {}), rows } }), {
      status: 200, headers: { "Content-Type": "application/json" },
    })) as unknown as typeof fetch
}

const noop = () => {}

// BI-18 part C review fix (SHOULD-FIX) R28: the arguments are the model's real create_chart
// call for "a progress bar of total visitors against a goal of 300,000" (stored chart u_e664e48e).
describe("the draft preview of a progress bar and a bullet graph", () => {
  it("compares a progress bar with the goal the model sent, not 'No goal to compare with'", async () => {
    previewOf({ id: "d", title: "Total visitors vs goal 300,000", kind: "progress", mart: "m", sql: "", x: "", y: "v", source: "ui" }, [{ v: 278955 }])
    render(
      <ChartDraftCard
        args={{ title: "Total visitors vs goal 300,000", kind: "progress", mart: "mart_demo_map_points", measures: [{ metric: "total_visitors_demo" }], aggregate: "sum", goal: 300000, board: "b1" }}
        confirming={false} onConfirm={noop} onCancel={noop} onSavedInBuilder={noop}
      />,
    )
    await waitFor(() => expect(screen.getByRole("progressbar")).toBeTruthy())
    expect(screen.queryByText("No goal to compare with")).toBeNull()
    expect(screen.getByText("93%")).toBeTruthy()
  })

  it("hands a bullet tile the goal and bands the model sent, and keeps a def the server did send", () => {
    const def = chartDefFromArgs({ title: "B", kind: "bullet", mart: "m", dimension: "k", measures: ["v", "g"], goal: { measure: "g" }, bands: [4, 8, 12] })
    const spec = previewSpecWithDef({ id: "d", kind: "bullet" }, def)
    expect(spec.def).toMatchObject({ goal: { measure: "g" }, bands: [4, 8, 12] })
    const own = { goal: 5 }
    expect(previewSpecWithDef({ id: "d", def: own }, def).def).toBe(own)
  })
})
