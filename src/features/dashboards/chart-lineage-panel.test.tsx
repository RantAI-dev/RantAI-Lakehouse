import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock } from "bun:test"

import * as realAuth from "@/features/auth/auth-provider"

// The permission is the test's to set; the real module is put back afterwards.
const real = { ...realAuth }
let permitted = true
mock.module("@/features/auth/auth-provider", () => ({ ...real, useAuth: () => ({ hasPermission: (p: string) => permitted && p === "lineage:read" }) }))
afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
})

import { ChartLineagePanel } from "./chart-lineage-panel"
import { ReportingProvider } from "./reporting-context"

const originalFetch = global.fetch
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
let asked: string[] = []

const PATH = {
  chart: { id: "c_1", title: "Visitors by province" },
  steps: [
    { id: "chart:c_1", kind: "chart", label: "Visitors by province" },
    { id: "table:serving.mart_visits", kind: "gold", label: "serving.mart_visits", refreshedAt: "2026-10-20T03:00:00Z", refreshSource: "last Gold build" },
    { id: "table:silver.visits", kind: "silver", label: "silver.visits", notRecorded: true },
    { id: "connector:crm", kind: "connector", label: "CRM", refreshedAt: "2026-10-20T01:00:00Z", refreshSource: "last connector run" },
  ],
  edges: [],
  fields: [
    { field: "province", columns: [{ table: "silver.visits", column: "prov", via: "Gold build" }] },
    { field: "visitors", columns: [] },
  ],
  coverage: ["A table loaded outside the platform has no source shown."],
  supported: true,
}

beforeEach(() => {
  asked = []
  permitted = true
})
afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

function mount(respond: () => Response) {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    asked.push(String(input))
    return respond()
  }) as unknown as typeof fetch
  return render(
    <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday" }}>
      <ChartLineagePanel chart={{ id: "c_1", title: "Visitors by province" }} onOpenChange={() => {}} />
    </ReportingProvider>,
  )
}

describe("ChartLineagePanel (BI-14)", () => {
  it("lists the path from the chart to the connector, each step with its refresh time or 'Not recorded'", async () => {
    mount(() => json(PATH))
    await screen.findByText("serving.mart_visits")
    expect(asked).toEqual(["/api/dashboard/charts/c_1/lineage"])
    const steps = screen.getAllByRole("listitem").filter((li) => li.hasAttribute("data-step-kind"))
    expect(steps.map((s) => s.getAttribute("data-step-kind"))).toEqual(["chart", "gold", "silver", "connector"])
    expect(screen.getByText(/Refreshed 20 Oct 2026, 03:00 \(last Gold build\)/)).toBeTruthy()
    expect(screen.getByText(/Refreshed 20 Oct 2026, 01:00 \(last connector run\)/)).toBeTruthy()
    // The chart itself has no refresh record, and so does the Silver step.
    expect(screen.getAllByText("Not recorded")).toHaveLength(2)
  })

  it("lists the columns behind each field, and says a field has no mapping", async () => {
    mount(() => json(PATH))
    await screen.findByText("silver.visits.prov")
    expect(screen.getByText("No column mapping recorded")).toBeTruthy()
    expect(screen.getByText("A table loaded outside the platform has no source shown.")).toBeTruthy()
  })

  it("links to the lineage page focused on the chart", async () => {
    mount(() => json(PATH))
    const link = (await screen.findByText("Open in lineage")).closest("a")
    expect(link?.getAttribute("href")).toBe("/lineage?focus=chart%3Ac_1")
  })

  it("gives a person without lineage:read the sentence and asks for nothing", async () => {
    permitted = false
    mount(() => json(PATH))
    expect(await screen.findByText("Lineage needs the lineage permission; ask your administrator.")).toBeTruthy()
    expect(asked).toEqual([])
    expect(screen.queryByText("Open in lineage")).toBeNull()
  })

  it("gives the same sentence when the server refuses", async () => {
    mount(() => json({ error: "forbidden" }, 403))
    expect(await screen.findByText("Lineage needs the lineage permission; ask your administrator.")).toBeTruthy()
  })

  it("shows an honest failure message when the read fails", async () => {
    mount(() => json({ error: "Lineage is unavailable" }, 500))
    await waitFor(() => expect(screen.getByRole("button", { name: /retry|try again/i })).toBeTruthy())
    expect(screen.queryByText("serving.mart_visits")).toBeNull()
  })
})
