import { cleanup, render, screen } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock } from "bun:test"

import * as realAuth from "@/features/auth/auth-provider"

const real = { ...realAuth }
let permitted = true
mock.module("@/features/auth/auth-provider", () => ({ ...real, useAuth: () => ({ hasPermission: (p: string) => permitted && p === "lineage:read" }) }))
afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
})

import { UsedBy } from "./used-by"
import { usedByTable } from "./lineage-page"
import { KIND_LABEL } from "./lineage-graph"

const originalFetch = global.fetch
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
let asked: string[] = []
beforeEach(() => {
  asked = []
  permitted = true
})
afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})
function answer(body: unknown, status = 200) {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    asked.push(String(input))
    return json(body, status)
  }) as unknown as typeof fetch
}

describe("UsedBy (BI-14)", () => {
  it("lists the charts with their dashboard and how they read the table, and the count it hides", async () => {
    answer({
      supported: true, table: "serving.mart_visits", hiddenCount: 3,
      charts: [
        { chartId: "c1", chartTitle: "Visitors", dashboardId: "d1", dashboardTitle: "Tourism", via: "direct" },
        { chartId: "c2", chartTitle: "Stay", dashboardId: "d1", dashboardTitle: "Tourism", via: "sql_source" },
        { chartId: "c3", chartTitle: "Revenue", dashboardId: "d2", dashboardTitle: "Finance", via: "metric" },
      ],
    })
    render(<UsedBy table="serving.mart_visits" />)
    await screen.findByText("Visitors")
    expect(asked).toEqual(["/api/governance/lineage/used-by?table=serving.mart_visits"])
    expect(screen.getByText(/on Tourism, through a SQL source/)).toBeTruthy()
    expect(screen.getByText(/on Finance, through a metric/)).toBeTruthy()
    expect(screen.getByText("and 3 more you do not have access to")).toBeTruthy()
    expect(screen.getByRole("link", { name: "Visitors" }).getAttribute("href")).toBe("/dashboards?board=d1")
  })

  it("says no chart reads the table when the list and the hidden count are empty", async () => {
    answer({ supported: true, table: "serving.t", charts: [], hiddenCount: 0 })
    render(<UsedBy table="serving.t" />)
    expect(await screen.findByText("No chart reads this table")).toBeTruthy()
  })

  it("does not say none when charts exist that the person cannot see", async () => {
    answer({ supported: true, table: "serving.t", charts: [], hiddenCount: 2 })
    render(<UsedBy table="serving.t" />)
    expect(await screen.findByText("and 2 more you do not have access to")).toBeTruthy()
    expect(screen.queryByText("No chart reads this table")).toBeNull()
  })

  it("gives a person without lineage:read the sentence and asks for nothing", () => {
    permitted = false
    answer({})
    render(<UsedBy table="serving.t" />)
    expect(screen.getByText("Lineage needs the lineage permission; ask your administrator.")).toBeTruthy()
    expect(asked).toEqual([])
  })
})

describe("the lineage page and chart nodes (BI-14)", () => {
  it("asks Used by only for a table focus", () => {
    expect(usedByTable("serving.orders")).toBe("serving.orders")
    expect(usedByTable("table:serving.orders")).toBe("serving.orders")
    expect(usedByTable("chart:c_1")).toBe("")
    expect(usedByTable("")).toBe("")
  })

  it("names the chart, SQL source and metric nodes", () => {
    expect(KIND_LABEL.chart).toBe("Chart")
    expect(KIND_LABEL.sql_source).toBe("SQL source")
    expect(KIND_LABEL.metric).toBe("Metric")
  })
})
