import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"

import * as realAuth from "@/features/auth/auth-provider"

// As in chart-builder.test.tsx: a plain editor, and the real module put back afterwards.
const real = { ...realAuth }
mock.module("@/features/auth/auth-provider", () => ({ ...real, useAuth: () => ({ hasPermission: () => false }) }))
afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
})

import { dashboardService } from "@/services"
import { ChartBuilder, type ChartDef } from "./chart-builder"
import { ReportingProvider } from "./reporting-context"

// `BI-58`: the builder's new layout keeps every saved chart as it was (spec T5).
const originalFetch = global.fetch
const happyDOM = (window as unknown as { happyDOM?: { settings: { disableIframePageLoading: boolean } } }).happyDOM
if (happyDOM) happyDOM.settings.disableIframePageLoading = true

/** The fields of the stub source: two text columns, a date, a timestamp and three numbers. */
const FIELDS = {
  dimensions: ["provinsi", "category", "begins", "ends"],
  measures: ["visitors", "stay", "target"],
  calculated: [], hierarchies: [],
  columns: [
    { name: "provinsi", type: "String" }, { name: "category", type: "String" },
    { name: "begins", type: "Date" }, { name: "ends", type: "DateTime" },
    { name: "visitors", type: "UInt64" }, { name: "stay", type: "Float64" }, { name: "target", type: "Float64" },
  ],
}

let fetches: string[] = []
let previews: Record<string, unknown>[] = []

beforeEach(() => {
  fetches = []
  previews = []
  window.localStorage.clear()
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    fetches.push(url)
    const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
    if (url.includes("/specs/preview")) {
      if (init?.body) previews.push(JSON.parse(String(init.body)))
      return json({ error: "not in this test" }, 400)
    }
    if (url.includes("/api/dashboard/fields") && url.includes("mart=")) return json(FIELDS)
    if (url.includes("/api/dashboard/fields")) return json({ marts: [{ name: "mart_visits", rows: 1571564 }] })
    if (url.includes("/api/dashboard/sources")) return json({ sources: [] })
    if (url.includes("/api/dashboard/metrics") && !url.includes("usage")) return json({ metrics: [] })
    return json({ error: "not in this test" }, 500)
  }) as unknown as typeof fetch
})
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function renderBuilder(initial?: ChartDef, onSaved: (saved: ChartDef) => void = () => {}) {
  return render(
    <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday" }}>
      <ChartBuilder onSaved={onSaved} onCancel={() => {}} initial={initial} editId={initial ? "c_1" : undefined} />
    </ReportingProvider>,
  )
}

/** What travels over the wire: `undefined` fields are dropped, as `JSON.stringify` drops them. */
const wire = (v: unknown) => JSON.parse(JSON.stringify(v)) as Record<string, unknown>

/**
 * One saved chart of each family, written as the builder writes it (every
 * field it sends, in its own shape), so a save of the unchanged chart must
 * send exactly this back.
 */
const SAVED: Record<string, ChartDef> = {
  bar: {
    title: "Visitors by province", mart: "mart_visits", kind: "bar", dimension: "provinsi",
    measures: ["visitors"], aggregate: "sum", span: 1, board: "default", order: "desc", limit: 20,
  } as ChartDef,
  line: {
    title: "Visitors by month", mart: "mart_visits", kind: "line", dimension: "begins",
    measures: ["visitors", { column: "stay", aggregate: "avg" }], aggregate: "sum", span: 2, board: "default",
    breakdown: "category", order: "none", limit: 12, grain: "month",
  } as unknown as ChartDef,
  kpi: {
    title: "Visitors", kind: "kpi", mart: "mart_visits", measures: ["visitors"], aggregate: "sum", span: 1, board: "default",
    caption: "this month", compare: { kind: "previous", dateColumn: "begins", period: "month" }, goodDirection: "down",
  } as unknown as ChartDef,
  table: {
    title: "Province table", mart: "mart_visits", kind: "table", dimension: "provinsi",
    measures: ["visitors", { column: "stay", aggregate: "max" }], aggregate: "sum", span: 2, board: "default",
    order: "desc", limit: 50,
  } as unknown as ChartDef,
  gantt: {
    title: "Plan", mart: "mart_visits", kind: "gantt", dimension: "provinsi", span: 1, board: "default",
    breakdown: "category", start: "begins", end: "ends", progress: "stay",
  } as unknown as ChartDef,
  combined: {
    title: "Visitors and stay", mart: "mart_visits", kind: "bar", dimension: "provinsi",
    measures: ["visitors"], aggregate: "sum", span: 2, board: "default", order: "desc", limit: 20,
    layers: [{ mart: "mart_visits", dimension: "provinsi", measures: [{ column: "stay", aggregate: "avg" }], aggregate: "sum", draw: "line", axis: "right" }],
  } as unknown as ChartDef,
}

describe("ChartBuilder keeps a saved chart as it was (BI-58, spec T5)", () => {
  for (const [family, def] of Object.entries(SAVED)) {
    it(`saves an unchanged ${family} chart back as the same definition`, async () => {
      const save = spyOn(dashboardService, "saveChart").mockResolvedValue({ id: "c_1" })
      let saved: ChartDef | null = null
      try {
        renderBuilder(def, (d) => { saved = d })
        // The source's columns are loaded once its own picks are back on screen; the preview asks for them.
        await waitFor(() => expect(previews.length).toBeGreaterThan(0))
        fireEvent.click(screen.getByRole("button", { name: "Save changes" }))
        await waitFor(() => expect(save).toHaveBeenCalledTimes(1))
        const [payload, edit] = save.mock.calls[0]!
        expect(edit).toBe(true)
        expect(wire(payload)).toEqual({ ...wire(def), id: "c_1" })
        await waitFor(() => expect(saved).not.toBeNull())
      } finally {
        save.mockRestore()
      }
    })
  }
})
