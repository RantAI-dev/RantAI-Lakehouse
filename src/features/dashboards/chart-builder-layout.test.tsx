import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"

import * as realAuth from "@/features/auth/auth-provider"
import * as realEchartModule from "./echart"

// As in chart-builder.test.tsx: a plain editor, and the real module put back afterwards.
const real = { ...realAuth }
mock.module("@/features/auth/auth-provider", () => ({ ...real, useAuth: () => ({ hasPermission: () => false }) }))
// ECharts draws on a canvas, which the test DOM does not have; each chart is a marker with its kind.
const realEchart = { ...realEchartModule }
mock.module("./echart", () => ({
  EChart: (props: { option: { series?: { type?: string }[] } }) => <div data-testid="chart" data-type={props.option.series?.[0]?.type ?? ""} />,
}))
afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
  mock.module("./echart", () => realEchart)
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
/** The preview route's answer: an error unless a test draws a chart. */
let previewAnswer: ((body: Record<string, unknown>) => { status: number; body: unknown }) | null = null

beforeEach(() => {
  fetches = []
  previews = []
  previewAnswer = null
  window.localStorage.clear()
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    fetches.push(url)
    const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
    if (url.includes("/specs/preview")) {
      const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : {}
      previews.push(body)
      if (previewAnswer) {
        const answer = previewAnswer(body)
        return json(answer.body, answer.status)
      }
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

/** A bar-like preview answer over the stub's rows: the X column and the first value, as the preview route returns them. */
function drawRows(body: Record<string, unknown>) {
  const x = String(body.dimension)
  const y = String((body.measures as unknown[])[0])
  return {
    status: 200,
    body: {
      spec: { id: "preview", title: "Preview", kind: body.kind, mart: "mart_visits", sql: "", x, y, source: "ui" },
      result: { columns: [x, y], rows: [{ [x]: "2024-01-01", [y]: 1571564 }, { [x]: "2024-02-01", [y]: 1204311 }] },
    },
  }
}

describe("the data panel (BI-58 T2, spec T4)", () => {
  async function pickSource() {
    renderBuilder()
    const panel = screen.getByRole("complementary", { name: "Data" })
    fireEvent.click(within(panel).getByRole("combobox"))
    fireEvent.click(await screen.findByRole("option", { name: /mart_visits/ }))
    await within(panel).findByRole("group", { name: "Dates" })
    return panel
  }

  it("lists the source's columns as Dates, Text and Numbers", async () => {
    const panel = await pickSource()
    const names = (group: string) => [...within(panel).getByRole("group", { name: group }).querySelectorAll("button")].map((b) => b.getAttribute("aria-label"))
    expect(names("Dates")).toEqual(["begins", "ends"])
    expect(names("Text")).toEqual(["provinsi", "category"])
    expect(names("Numbers")).toEqual(["visitors", "stay", "target"])
  })

  it("gives a preview from a source, one click on a date column and one on a number column", async () => {
    previewAnswer = drawRows
    const panel = await pickSource()
    fireEvent.click(within(panel).getByRole("button", { name: "begins" }))
    fireEvent.click(within(panel).getByRole("button", { name: "visitors" }))
    // The chips say where each column went, and no menu opened on the way.
    expect(within(panel).getByRole("button", { name: "begins, X axis" })).toBeTruthy()
    expect(within(panel).getByRole("button", { name: "visitors, Value" })).toBeTruthy()
    expect(screen.queryByRole("menu")).toBeNull()
    await waitFor(() => expect(previews.some((b) => b.dimension === "begins" && JSON.stringify(b.measures) === '["visitors"]')).toBe(true))
    const preview = screen.getByRole("region", { name: "Preview" })
    await waitFor(() => expect(within(preview).getByTestId("chart")).toBeTruthy())
    expect(preview.textContent).not.toContain("Preview unavailable")
  })

  it("opens a menu of the uses the chart offers for a column once its slot is taken", async () => {
    const panel = await pickSource()
    fireEvent.click(within(panel).getByRole("button", { name: "provinsi" }))
    expect(within(panel).getByRole("button", { name: "provinsi, X axis" })).toBeTruthy()
    // The X axis is taken: a second text column asks.
    fireEvent.click(within(panel).getByRole("button", { name: "category" }))
    const menu = await screen.findByRole("menu")
    expect([...menu.querySelectorAll("[role=menuitem]")].map((i) => i.textContent)).toEqual(["Use as X axis", "Use as split", "Use as filter"])
    fireEvent.click(within(menu).getByRole("menuitem", { name: "Use as split" }))
    await waitFor(() => expect(within(panel).getByRole("button", { name: "category, Split" })).toBeTruthy())
  })
})
