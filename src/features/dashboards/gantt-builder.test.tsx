import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock } from "bun:test"

import * as realAuth from "@/features/auth/auth-provider"

// As in chart-builder.test.tsx: a plain editor, and the real module put back afterwards.
const real = { ...realAuth }
mock.module("@/features/auth/auth-provider", () => ({ ...real, useAuth: () => ({ hasPermission: () => false }) }))
afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
})

import { ChartBuilder, KIND_GROUPS, type ChartDef } from "./chart-builder"
import { ReportingProvider } from "./reporting-context"

// BI-16 part C1, T3 (plan D1): the builder's Gantt fields round-trip through a saved definition.
const originalFetch = global.fetch
const happyDOM = (window as unknown as { happyDOM?: { settings: { disableIframePageLoading: boolean } } }).happyDOM
if (happyDOM) happyDOM.settings.disableIframePageLoading = true

let previews: Record<string, unknown>[] = []

beforeEach(() => {
  previews = []
  window.localStorage.clear()
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
    if (url.includes("/specs/preview")) {
      if (init?.body) previews.push(JSON.parse(String(init.body)))
      return json({ error: "not in this test" }, 400)
    }
    if (url.includes("/api/dashboard/fields") && url.includes("mart=")) {
      return json({
        dimensions: ["task", "team", "begins", "ends"], measures: ["done_pct"], calculated: [], hierarchies: [],
        columns: [
          { name: "task", type: "String" }, { name: "team", type: "String" }, { name: "begins", type: "Date" },
          { name: "ends", type: "DateTime" }, { name: "done_pct", type: "Float64" },
        ],
      })
    }
    if (url.includes("/api/dashboard/fields")) return json({ marts: [{ name: "mart_plan", rows: 10 }] })
    if (url.includes("/api/dashboard/sources")) return json({ sources: [] })
    if (url.includes("/api/dashboard/metrics") && !url.includes("usage")) return json({ metrics: [] })
    return json({ error: "not in this test" }, 500)
  }) as unknown as typeof fetch
})
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function renderBuilder(initial: ChartDef) {
  return render(
    <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday" }}>
      <ChartBuilder open onOpenChange={() => {}} hideTrigger onSaved={() => {}} initial={initial} editId="c_1" />
    </ReportingProvider>,
  )
}

describe("ChartBuilder Gantt (BI-16 part C1)", () => {
  it("offers the Gantt type", () => {
    expect(KIND_GROUPS.flatMap((g) => g.items.map((i) => i.value))).toContain("gantt")
  })

  it("reopens a Gantt chart with its task, group, start, end and progress, and sends the same definition back with no measure, order or limit", async () => {
    renderBuilder({ title: "Plan", kind: "gantt", mart: "mart_plan", dimension: "task", breakdown: "team", start: "begins", end: "ends", progress: "done_pct" })
    await waitFor(() => expect(previews.length).toBeGreaterThan(0))
    const body = previews[previews.length - 1]
    expect(body).toMatchObject({ kind: "gantt", dimension: "task", breakdown: "team", start: "begins", end: "ends", progress: "done_pct" })
    for (const field of ["measures", "aggregate", "order", "limit", "lat", "lon"]) expect(body[field]).toBeUndefined()
    expect(screen.getByText("Task column")).toBeTruthy()
    expect(screen.getByLabelText("Start column")).toBeTruthy()
    await waitFor(() => expect(screen.getByLabelText("Progress column").textContent).toContain("done_pct"))
  })

  it("asks for the start and the end before it can be previewed", async () => {
    renderBuilder({ title: "Plan", kind: "gantt", mart: "mart_plan", dimension: "task" })
    await screen.findByLabelText("Start column")
    await screen.findByText(/Fill in Start column/)
    expect(previews).toHaveLength(0)
  })
})
