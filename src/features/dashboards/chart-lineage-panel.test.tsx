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

describe("ChartLineagePanel (BI-14, look BI-71)", () => {
  it("reads the path from the source down to the chart, the reverse of the API's order", async () => {
    mount(() => json(PATH))
    await screen.findByText("serving.mart_visits")
    expect(asked).toEqual(["/api/dashboard/charts/c_1/lineage"])
    const steps = screen.getAllByRole("listitem").filter((li) => li.hasAttribute("data-step-kind"))
    expect(steps.map((s) => s.getAttribute("data-step-kind"))).toEqual(["connector", "silver", "gold", "chart"])
    // The chart's title is the sheet's description already; its step says where the reader is.
    expect(steps[3].textContent).toContain("This chart")
  })

  it("shows a refresh time only on a step that has one, and never 'Not recorded'", async () => {
    mount(() => json(PATH))
    await screen.findByText("serving.mart_visits")
    const step = (kind: string) => screen.getAllByRole("listitem").find((li) => li.getAttribute("data-step-kind") === kind)!
    // The exact time and its record are read out (and shown in the tooltip).
    expect(step("gold").textContent).toContain("Refreshed 20 Oct 2026, 03:00 (last Gold build)")
    expect(step("connector").textContent).toContain("Refreshed 20 Oct 2026, 01:00 (last connector run)")
    expect(step("silver").textContent).not.toContain("Refreshed")
    expect(step("chart").textContent).not.toContain("Refreshed")
    expect(screen.queryByText(/Not recorded/)).toBeNull()
  })

  it("says what is missing once, with the server's coverage in the help hint", async () => {
    mount(() => json(PATH))
    await screen.findByText("serving.mart_visits")
    expect(screen.getAllByText("Only recorded times and columns are shown")).toHaveLength(1)
    expect(screen.queryByText("No column mapping recorded")).toBeNull()
    expect(screen.getByRole("button", { name: "A table loaded outside the platform has no source shown." })).toBeTruthy()
  })

  it("says nothing is recorded yet when no step has a time and no field a column", async () => {
    const bare = {
      ...PATH,
      steps: PATH.steps.map(({ id, kind, label }) => ({ id, kind, label, notRecorded: kind !== "chart" })),
      fields: PATH.fields.map((f) => ({ ...f, columns: [] })),
    }
    mount(() => json(bare))
    expect(await screen.findByText("No refresh times or column mappings recorded yet")).toBeTruthy()
    expect(screen.queryByText("Only recorded times and columns are shown")).toBeNull()
  })

  it("puts the fields on the chart step, a mapped one as field ← column with the full name for the reader", async () => {
    mount(() => json(PATH))
    const fields = await screen.findByRole("list", { name: "Fields of this chart" })
    const chips = Array.from(fields.querySelectorAll(":scope > li")).map((li) => li.textContent)
    expect(chips[0]).toContain("province←prov")
    expect(chips[0]).toContain("from silver.visits.prov · Gold build")
    // An unmapped field is only its name: the missing mapping is said once, below.
    expect(chips[1]).toBe("visitors")
  })

  it("links to the lineage page focused on the chart", async () => {
    mount(() => json(PATH))
    const link = (await screen.findByText("Open in lineage")).closest("a")
    expect(link?.getAttribute("href")).toBe("/lineage?focus=chart%3Ac_1")
  })

  it("loads in the route's shape", async () => {
    mount(() => json(PATH))
    expect(screen.getByRole("status", { name: "Loading the path" })).toBeTruthy()
    await screen.findByText("serving.mart_visits")
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

  it("says so when the deployment does not record lineage", async () => {
    mount(() => json({ ...PATH, steps: [], fields: [], coverage: [], supported: false }))
    expect(await screen.findByText("Lineage is not recorded on this deployment.")).toBeTruthy()
    expect(screen.queryByText("Open in lineage")).toBeNull()
  })

  it("shows an honest failure message when the read fails", async () => {
    mount(() => json({ error: "Lineage is unavailable" }, 500))
    await waitFor(() => expect(screen.getByRole("button", { name: /retry|try again/i })).toBeTruthy())
    expect(screen.queryByText("serving.mart_visits")).toBeNull()
  })
})
