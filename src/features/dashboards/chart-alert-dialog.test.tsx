import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { alertNumberShape, chartHasLatestPoint } from "./chart-alert"
import { ChartAlertDialog, NO_LATEST_POINT_SENTENCE, NO_SINGLE_NUMBER_SENTENCE, OWN_ADDRESS_SENTENCE, READ_AS_YOU_SENTENCE } from "./chart-alert-dialog"

const originalFetch = global.fetch
type Call = { url: string; method: string; body: Record<string, unknown> | undefined }

const mineRule = { id: "al_9", name: "Alert: Visitors", type: "alert", chart: "c1", condition: "above", threshold: 100, channel: "email", target: "me@company.example", enabled: true }

/**
 * Records the writes (`calls`). The dialog's own read of `/api/alerts/mine` is
 * answered with `mine` and kept out of `calls`, so each test counts only what
 * it saves or removes (BI-43).
 */
function mockApi(status = 200, body: unknown = { ok: true, rule: { id: "al_1" } }, mine: unknown[] = []) {
  const calls: Call[] = []
  global.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    if (String(input).startsWith("/api/alerts/mine")) {
      return new Response(JSON.stringify({ rules: mine }), { status: 200, headers: { "Content-Type": "application/json" } })
    }
    calls.push({ url: String(input), method: init?.method ?? "GET", body: init?.body ? JSON.parse(String(init.body)) : undefined })
    return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
  }) as unknown as typeof fetch
  return calls
}

const kpi = { id: "c1", title: "Visitors", kind: "kpi", measures: ["visitors"] }
const line = { id: "c2", title: "Trend", kind: "line", measures: ["visitors", "bookings"] }
const pie = { id: "c3", title: "Share", kind: "pie", measures: [] }

function open(chart = kpi, extra: { canWriteAlerts?: boolean; filters?: unknown[] } = {}) {
  render(
    <ChartAlertDialog open onOpenChange={() => {}} chart={chart} filters={extra.filters ?? []}
      userEmail="me@company.example" canWriteAlerts={extra.canWriteAlerts ?? false} />,
  )
}

beforeEach(() => cleanup())
afterEach(() => { global.fetch = originalFetch; cleanup() })

describe("alertNumberShape", () => {
  it("gives a number to a KPI, a gauge and a line, area, bar or combo chart, and to nothing else", () => {
    expect(alertNumberShape("kpi")).toBe("single")
    expect(alertNumberShape("gauge")).toBe("single")
    for (const k of ["line", "area", "bar", "combo"]) expect(alertNumberShape(k)).toBe("series")
    for (const k of ["pie", "table", "heatmap", "stacked"]) expect(alertNumberShape(k)).toBe("none")
  })
})

describe("chartHasLatestPoint", () => {
  it("mirrors the server: a value sort has none, a sort by the dimension or no order has one", () => {
    // BI-19 owner QA 2026-10-28, QA-6
    expect(chartHasLatestPoint({ dimension: "provinsi", sort: { by: "sum_visitors" } })).toBe(false)
    expect(chartHasLatestPoint({ dimension: "visit_date", sort: null })).toBe(true)
    expect(chartHasLatestPoint({ dimension: "visit_date", sort: { by: "visit_date" } })).toBe(true)
    expect(chartHasLatestPoint({ dimension: "d", order: "desc" })).toBe(false)
    expect(chartHasLatestPoint({ dimension: "d", order: "none" })).toBe(true)
    expect(chartHasLatestPoint(undefined)).toBe(true)
  })
})

describe("ChartAlertDialog", () => {
  it("leaves the top-N __other column out of the series and labels the rest as the legend does", async () => {
    // BI-19 owner QA 2026-10-28, QA-7
    const calls = mockApi()
    open({ id: "c5", title: "Trend", kind: "line", measures: ["sum_visitors", "__other", "avg_visitors"], labels: { sum_visitors: "sum of visitors", avg_visitors: "avg of visitors" } } as typeof line)
    const series = screen.getByRole("combobox", { name: /Series/ })
    expect(series.textContent).toContain("sum of visitors")
    fireEvent.click(series)
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["sum of visitors", "avg of visitors"])
    fireEvent.change(screen.getByLabelText("Line"), { target: { value: "5" } })
    fireEvent.click(screen.getByRole("button", { name: "Save alert" }))
    await waitFor(() => expect(calls.length).toBe(1))
    expect(calls[0].body).toMatchObject({ series: "sum_visitors" })
  })

  it("offers only 'returns any rows' for a chart sorted by value, with the reason, and hides the series", async () => {
    const calls = mockApi()
    open({ id: "c4", title: "By province", kind: "bar", measures: ["sum_visitors"], latestPoint: false } as typeof line)
    const when = screen.getByRole("combobox", { name: "When" })
    expect(when.textContent).toContain("the chart returns any rows")
    fireEvent.click(when)
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["the chart returns any rows"])
    // Owner QA 2026-11-16: one short line, the reason behind its ⓘ (named by it, shown on a press).
    expect(screen.getByText(/Only “returns any rows” for this chart/)).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: NO_LATEST_POINT_SENTENCE }))
    expect(await screen.findByText(NO_LATEST_POINT_SENTENCE)).toBeTruthy()
    expect(screen.queryByRole("combobox", { name: /Series/ })).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Save alert" }))
    await waitFor(() => expect(calls.length).toBe(1))
    expect(calls[0].body).toMatchObject({ chart: "c4", condition: "any_rows" })
    expect(calls[0].body?.series).toBeUndefined()
  })

  it("saves a KPI alert with the chart, the condition, the line, the filters on screen and the person's own address", async () => {
    const calls = mockApi()
    open(kpi, { filters: [{ column: "region", values: ["Bali"] }] })
    fireEvent.change(screen.getByLabelText("Line"), { target: { value: "100" } })
    expect(screen.getByText("Read as you, with the 1 dashboard filter on screen now")).toBeTruthy()
    expect(screen.getByRole("button", { name: READ_AS_YOU_SENTENCE })).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Save alert" }))
    await waitFor(() => expect(calls.length).toBe(1))
    expect(calls[0].method).toBe("POST")
    expect(calls[0].url).toBe("/api/alerts")
    expect(calls[0].body).toMatchObject({
      type: "alert", chart: "c1", condition: "above", threshold: 100, channel: "email",
      target: "me@company.example", filters: [{ column: "region", values: ["Bali"] }],
    })
    expect(calls[0].body?.series).toBeUndefined()
  })

  it("asks for the series of a line chart and sends the chosen one", async () => {
    const calls = mockApi()
    open(line)
    fireEvent.change(screen.getByLabelText("Line"), { target: { value: "5" } })
    chooseOption(screen.getByRole("combobox", { name: /Series/ }), "bookings")
    fireEvent.click(screen.getByRole("button", { name: "Save alert" }))
    await waitFor(() => expect(calls.length).toBe(1))
    expect(calls[0].body).toMatchObject({ chart: "c2", series: "bookings", condition: "above" })
  })

  it("offers only 'returns any rows' for a chart with no single number, and sends no line", async () => {
    const calls = mockApi()
    open(pie)
    expect(screen.queryByLabelText("Line")).toBeNull()
    expect(screen.getByText(/Only “returns any rows” for this chart/)).toBeTruthy()
    expect(screen.getByRole("button", { name: NO_SINGLE_NUMBER_SENTENCE })).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Save alert" }))
    await waitFor(() => expect(calls.length).toBe(1))
    expect(calls[0].body).toMatchObject({ chart: "c3", condition: "any_rows" })
    expect(calls[0].body?.threshold).toBeUndefined()
  })

  it("fixes the target to the person's own address without the alert permission, and lets the permission change it", () => {
    open()
    const target = screen.getByLabelText("Send to") as HTMLInputElement
    expect(target.readOnly).toBe(true)
    expect(screen.getByText(/Only your own address/)).toBeTruthy()
    expect(screen.getByRole("button", { name: OWN_ADDRESS_SENTENCE })).toBeTruthy()
    cleanup()
    open(kpi, { canWriteAlerts: true })
    expect((screen.getByLabelText("Send to") as HTMLInputElement).readOnly).toBe(false)
    expect(screen.queryByRole("button", { name: OWN_ADDRESS_SENTENCE })).toBeNull()
  })

  it("refuses a missing line before sending, and shows the server's sentence when it refuses", async () => {
    const calls = mockApi(400, { error: "This chart has no single number." })
    open()
    fireEvent.click(screen.getByRole("button", { name: "Save alert" }))
    expect(await screen.findByText("Enter the line as a number.")).toBeTruthy()
    expect(calls.length).toBe(0)
    fireEvent.change(screen.getByLabelText("Line"), { target: { value: "1" } })
    fireEvent.click(screen.getByRole("button", { name: "Save alert" }))
    expect(await screen.findByText(/no single number/)).toBeTruthy()
  })

  it("lists the person's own alerts on the chart and removes one only after a confirmation", async () => {
    // BI-43 D1
    const calls = mockApi(200, { ok: true }, [mineRule])
    open()
    expect(await screen.findByText("Your alerts on this chart")).toBeTruthy()
    // Owner QA 2026-11-16: the person's own address is not repeated on the row.
    expect(screen.getByText(/· above 100$/)).toBeTruthy()
    expect(screen.queryByText(/me@company\.example/)).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Remove Alert: Visitors" }))
    expect(calls.length).toBe(0)
    expect(await screen.findByText("Remove alert?")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Remove alert" }))
    await waitFor(() => expect(calls.length).toBe(1))
    expect(calls[0].method).toBe("DELETE")
    expect(calls[0].url).toBe("/api/alerts?id=al_9")
  })

  it("names an alert's other address on its row, with the line in thousands", async () => {
    mockApi(200, { ok: true }, [{ ...mineRule, threshold: 1250000, target: "ops@company.example" }])
    open()
    expect(await screen.findByText(/· above 1,250,000 · to ops@company\.example$/)).toBeTruthy()
    expect(screen.getByText("New alert")).toBeTruthy()
  })

  it("has one way to close: no Cancel next to the dialog's own close", () => {
    // Owner QA 2026-11-16: X and Cancel both closed the dialog.
    mockApi()
    open()
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull()
    expect(screen.getByRole("button", { name: "Close" })).toBeTruthy()
    expect(screen.getByText("Read as you")).toBeTruthy()
  })

  it("gives the When and Series choices the full width of the dialog, like the other fields", () => {
    mockApi()
    open(line)
    for (const name of ["When", /Series/]) expect(screen.getByRole("combobox", { name }).className).toContain("w-full")
  })

  it("shows no section when the person has no alert on the chart", async () => {
    mockApi()
    open()
    await waitFor(() => expect(screen.getByLabelText("Name")).toBeTruthy())
    expect(screen.queryByText("Your alerts on this chart")).toBeNull()
    expect(screen.queryByText("New alert")).toBeNull()
  })
})
