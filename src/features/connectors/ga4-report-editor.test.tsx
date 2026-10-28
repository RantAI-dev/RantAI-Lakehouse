import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import * as React from "react"
import { Ga4ReportEditor } from "./ga4-report-editor"

/** Slice 3 review BLOCKER 9: a GA4 report is defined by choosing the property's own dimensions and metrics. */

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

function stub(objects: unknown[]): { bodies: unknown[] } {
  const bodies: unknown[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    if (String(input) !== "/api/connectors/c1/discovery/list") throw new Error(`unexpected fetch: ${String(input)}`)
    bodies.push(init?.body ? JSON.parse(init.body as string) : undefined)
    return json({ kind: "list", state: "done", objects })
  }) as unknown as typeof fetch
  return { bodies }
}

const FIELDS = [
  { name: "country", kind: "dimension" },
  { name: "date", kind: "dimension" },
  { name: "yearMonth", kind: "dimension" },
  { name: "sessions", kind: "metric" },
  { name: "activeUsers", kind: "metric" },
]

function Harness({ reports = [], onAdd = () => {}, onRemove = () => {} }: { reports?: string[]; onAdd?: (t: string, c: string, k: string[]) => void; onRemove?: (t: string) => void }) {
  return <Ga4ReportEditor connectorId="c1" reports={reports} onAdd={onAdd} onRemove={onRemove} pollMs={5} />
}

describe("Ga4ReportEditor", () => {
  it("reads the property's dimensions and metrics through discovery before it offers any", async () => {
    const { bodies } = stub(FIELDS)
    render(<Harness />)
    expect(screen.queryByLabelText("country")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Load dimensions and metrics" }))
    expect(await screen.findByLabelText("country")).toBeDefined()
    expect(bodies).toEqual([{ schema: "fields" }])
    expect(screen.getByLabelText("sessions")).toBeDefined()
  })

  it("builds a report from the choices, adds the date as its cursor, and clears the draft", async () => {
    stub(FIELDS)
    const added: [string, string, string[]][] = []
    render(<Harness onAdd={(t, c, k) => added.push([t, c, k])} />)
    fireEvent.click(screen.getByRole("button", { name: "Load dimensions and metrics" }))
    fireEvent.click(await screen.findByLabelText("country"))
    fireEvent.click(screen.getByLabelText("sessions"))
    fireEvent.click(screen.getByLabelText("activeUsers"))
    expect(screen.getByTestId("ga4-cursor").textContent).toMatch(/date is always part/)
    const add = screen.getByRole("button", { name: "Add report" }) as HTMLButtonElement
    expect(add.disabled).toBe(true) // no name yet
    fireEvent.change(screen.getByLabelText("Report name"), { target: { value: "traffic" } })
    expect(add.disabled).toBe(false)
    fireEvent.click(add)
    expect(added).toEqual([["traffic = country, date | sessions, activeUsers", "date", ["country", "date"]]])
    expect((screen.getByLabelText("Report name") as HTMLInputElement).value).toBe("")
  })

  it("uses a chosen time dimension as the cursor instead of adding the date", async () => {
    stub(FIELDS)
    const added: [string, string, string[]][] = []
    render(<Harness onAdd={(t, c, k) => added.push([t, c, k])} />)
    fireEvent.click(screen.getByRole("button", { name: "Load dimensions and metrics" }))
    fireEvent.click(await screen.findByLabelText("yearMonth"))
    fireEvent.click(screen.getByLabelText("sessions"))
    fireEvent.change(screen.getByLabelText("Report name"), { target: { value: "monthly" } })
    expect(screen.getByTestId("ga4-cursor").textContent).toMatch(/cut by yearMonth/)
    fireEvent.click(screen.getByRole("button", { name: "Add report" }))
    expect(added).toEqual([["monthly = yearMonth | sessions", "yearMonth", ["yearMonth"]]])
  })

  it("refuses a report with no metric, a bad name, or a name already used", async () => {
    stub(FIELDS)
    render(<Harness reports={["traffic = country, date | sessions"]} />)
    fireEvent.click(screen.getByRole("button", { name: "Load dimensions and metrics" }))
    fireEvent.click(await screen.findByLabelText("country"))
    fireEvent.change(screen.getByLabelText("Report name"), { target: { value: "x" } })
    expect(screen.getByText("Choose at least one metric.")).toBeDefined()
    fireEvent.click(screen.getByLabelText("sessions"))
    fireEvent.change(screen.getByLabelText("Report name"), { target: { value: "1 bad" } })
    expect(screen.getByText(/Name the report with letters/)).toBeDefined()
    fireEvent.change(screen.getByLabelText("Report name"), { target: { value: "traffic" } })
    expect(screen.getByText("A report with this name exists.")).toBeDefined()
    expect((screen.getByRole("button", { name: "Add report" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("lists the connector's reports and removes one", () => {
    const removed: string[] = []
    render(<Harness reports={["traffic = country, date | sessions"]} onRemove={(t) => removed.push(t)} />)
    expect(screen.getByText("traffic = country, date | sessions")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Remove" }))
    expect(removed).toEqual(["traffic = country, date | sessions"])
  })

  it("says plainly when the property cannot be read, and offers no choices", async () => {
    global.fetch = mock(async () => json({ kind: "list", state: "failed", errorClass: "no_permission", reason: "The connector's account has no permission to read this at the source." })) as unknown as typeof fetch
    render(<Harness />)
    fireEvent.click(screen.getByRole("button", { name: "Load dimensions and metrics" }))
    await waitFor(() => expect(screen.getByText(/Cannot read the property · The connector's account has no permission/)).toBeDefined())
    expect(screen.queryByRole("button", { name: "Add report" })).toBeNull()
  })
})
