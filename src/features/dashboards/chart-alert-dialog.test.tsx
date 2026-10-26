import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import { alertNumberShape } from "./chart-alert"
import { ChartAlertDialog, OWN_ADDRESS_SENTENCE } from "./chart-alert-dialog"

const originalFetch = global.fetch
type Call = { url: string; method: string; body: Record<string, unknown> | undefined }

function mockApi(status = 200, body: unknown = { ok: true, rule: { id: "al_1" } }) {
  const calls: Call[] = []
  global.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
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

describe("ChartAlertDialog", () => {
  it("saves a KPI alert with the chart, the condition, the line, the filters on screen and the person's own address", async () => {
    const calls = mockApi()
    open(kpi, { filters: [{ column: "region", values: ["Bali"] }] })
    fireEvent.change(screen.getByLabelText("Line"), { target: { value: "100" } })
    expect(screen.getByText(/Read with the 1 dashboard filter on screen now/)).toBeTruthy()
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
    fireEvent.change(screen.getByLabelText(/Series/), { target: { value: "bookings" } })
    fireEvent.click(screen.getByRole("button", { name: "Save alert" }))
    await waitFor(() => expect(calls.length).toBe(1))
    expect(calls[0].body).toMatchObject({ chart: "c2", series: "bookings", condition: "above" })
  })

  it("offers only 'returns any rows' for a chart with no single number, and sends no line", async () => {
    const calls = mockApi()
    open(pie)
    expect(screen.queryByLabelText("Line")).toBeNull()
    expect(screen.getByText(/can only ask whether it returns any rows/)).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Save alert" }))
    await waitFor(() => expect(calls.length).toBe(1))
    expect(calls[0].body).toMatchObject({ chart: "c3", condition: "any_rows" })
    expect(calls[0].body?.threshold).toBeUndefined()
  })

  it("fixes the target to the person's own address without the alert permission, and lets the permission change it", () => {
    open()
    const target = screen.getByLabelText("Send to") as HTMLInputElement
    expect(target.readOnly).toBe(true)
    expect(screen.getByText(OWN_ADDRESS_SENTENCE)).toBeTruthy()
    cleanup()
    open(kpi, { canWriteAlerts: true })
    expect((screen.getByLabelText("Send to") as HTMLInputElement).readOnly).toBe(false)
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
})
