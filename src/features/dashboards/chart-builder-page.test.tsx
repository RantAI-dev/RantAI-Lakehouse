// `WEB-7` T6 (D2/D4/D6): the chart builder is its own page.
//
// The page loads the board's level first (the server decides edit by the level
// on the dashboard, not by `dashboard:write` alone), loads a saved chart by id,
// and takes the assistant's draft from `sessionStorage` once. The board's
// parameter values come from the address and reach the preview only.

let pushed: string[] = []
let search = ""
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/b1/charts/new",
  useSearchParams: () => new URLSearchParams(search),
  useRouter: () => ({
    push: (href: string) => {
      pushed.push(href)
    },
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

// The page asks who is signed in for the one global grant the dashboard page
// also reads; the provider would fetch the session, so the hook is stubbed.
let permissions: string[] = []
import * as realAuth from "@/features/auth/auth-provider"
const real = { ...realAuth }
mock.module("@/features/auth/auth-provider", () => ({
  ...real,
  useAuth: () => ({
    user: { id: "u1" },
    hasPermission: (perm: string) => permissions.includes(perm),
    signOut: () => {},
    refresh: () => {},
    loading: false,
  }),
}))

// The chat's own provider is the app frame's and survives this navigation
// (`WEB-7` D6), so the page marks the assistant's step saved through it.
// `mock.module` outlives this file in `bun test src`, so the real exports are
// kept and put back in `afterAll` (`chart-builder.test.tsx`'s note).
let completed: { messageIndex: number; stepIndex: number; result: Record<string, unknown> }[] = []
import * as realCopilot from "@/features/copilot/use-copilot"
const realCopilotModule = { ...realCopilot }
mock.module("@/features/copilot/use-copilot", () => ({
  useCopilot: () => ({
    completeToolStep: (
      messageIndex: number,
      stepIndex: number,
      result: Record<string, unknown>
    ) => {
      completed.push({ messageIndex, stepIndex, result })
    },
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { dashboardService } from "@/services"
import type { ItemAccess } from "@/services/contracts/dashboards"
import { CHART_DRAFT_KEY, writeChartDraft } from "./chart-page"
import { ChartBuilderPage } from "./chart-builder-page"

afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
  mock.module("@/features/copilot/use-copilot", () => realCopilotModule)
})

const originalFetch = global.fetch
let bodies: { url: string; method: string; body: Record<string, unknown> }[] = []

beforeEach(() => {
  pushed = []
  search = ""
  permissions = []
  bodies = []
  completed = []
  window.localStorage.clear()
  window.sessionStorage.clear()
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const json = (body: unknown, status = 200) =>
      new Response(JSON.stringify(body), {
        status,
        headers: { "Content-Type": "application/json" },
      })
    if (url.includes("/specs/preview")) {
      if (init?.body) {
        bodies.push({ url, method: init?.method ?? "GET", body: JSON.parse(String(init.body)) })
      }
      // A preview the page does not need to draw for these assertions.
      return json({ error: "not in this test" }, 400)
    }
    if (url.includes("/api/dashboard/specs")) {
      if (init?.body) {
        bodies.push({ url, method: init?.method ?? "GET", body: JSON.parse(String(init.body)) })
      }
      return json({ chart: { id: "c_9" } })
    }
    if (url.includes("/api/dashboard/fields") && url.includes("mart=")) {
      return json({
        dimensions: ["provinsi", "category"],
        measures: ["visitors"],
        calculated: [],
        hierarchies: [],
        columns: [
          { name: "provinsi", type: "String" },
          { name: "category", type: "String" },
          { name: "visitors", type: "UInt64" },
        ],
      })
    }
    if (url.includes("/api/dashboard/fields")) return json({ marts: [{ name: "mart_visits", rows: 10 }] })
    if (url.includes("/api/dashboard/sources")) return json({ sources: [] })
    if (url.includes("/api/dashboard/metrics") && !url.includes("usage")) return json({ metrics: [] })
    return json({ error: "not in this test" }, 500)
  }) as unknown as typeof fetch
})

afterEach(() => {
  cleanup()
  mock.restore()
  global.fetch = originalFetch
})

function access(level: string): ItemAccess {
  return {
    kind: "board",
    id: "b1",
    yourLevel: level,
    restricted: false,
    rules: [],
    inherited: [],
    restrictedAbove: [],
  }
}

/** The reads the page makes: the board's level, the boards, and (edit) the chart. */
function stubPage(level: string, chart: Record<string, unknown> | null = null) {
  spyOn(dashboardService, "getAccess").mockResolvedValue(access(level))
  spyOn(dashboardService, "listBoards").mockResolvedValue([
    { id: "default", name: "Main" },
    { id: "b1", name: "Sales" },
  ])
  spyOn(dashboardService, "getChart").mockResolvedValue(
    chart as Awaited<ReturnType<typeof dashboardService.getChart>>
  )
}

const DRAFT = {
  title: "Visitors by province",
  kind: "bar" as const,
  mart: "mart_visits",
  dimension: "provinsi",
  measures: ["visitors"],
  aggregate: "sum",
}

describe("the chart builder page (WEB-7 D2/D4/D6)", () => {
  it("opens the form for a board the caller may edit through its rules alone", async () => {
    stubPage("edit")
    render(<ChartBuilderPage boardId="b1" />)

    // No `dashboard:write` anywhere: the level on this board is what counts.
    expect(await screen.findByRole("heading", { level: 1 })).toHaveProperty(
      "textContent",
      "New chart"
    )
    expect(screen.getByRole("button", { name: "Create chart" })).toBeTruthy()
  })

  it("shows You don't have access for a board the caller may only view", async () => {
    stubPage("view")
    render(<ChartBuilderPage boardId="b1" />)

    expect(await screen.findByText("You don't have access")).toBeDefined()
    expect(screen.queryByRole("heading", { level: 1 })).toBeNull()
  })

  it("fills the form from the chart the edit address names", async () => {
    stubPage("edit", { id: "c1", title: "Saved", board: "b1", def: DRAFT })
    render(<ChartBuilderPage boardId="b1" chartId="c1" />)

    expect(await screen.findByRole("heading", { level: 1 })).toHaveProperty(
      "textContent",
      "Edit chart"
    )
    await waitFor(() =>
      expect((screen.getByLabelText(/^Title/) as HTMLInputElement).value).toBe(
        "Visitors by province"
      )
    )
    expect(screen.getByRole("button", { name: "Save changes" })).toBeTruthy()
  })

  it("shows the not-found state for a chart id nothing has", async () => {
    spyOn(dashboardService, "getAccess").mockResolvedValue(access("edit"))
    spyOn(dashboardService, "listBoards").mockResolvedValue([])
    spyOn(dashboardService, "getChart").mockResolvedValue(null)
    render(<ChartBuilderPage boardId="b1" chartId="gone" />)

    expect(await screen.findByText("Chart not found")).toBeDefined()
    expect(screen.getByText("Back to the dashboard").closest("a")?.getAttribute("href")).toBe(
      "/dashboards/b1"
    )
  })

  it("takes the assistant's draft once, for its own board", async () => {
    stubPage("edit")
    writeChartDraft({ board: "b1", def: DRAFT })
    render(<ChartBuilderPage boardId="b1" />)

    await waitFor(() =>
      expect((screen.getByLabelText(/^Title/) as HTMLInputElement).value).toBe(
        "Visitors by province"
      )
    )
    // Read once: the key is gone, so a reload opens the page empty.
    expect(window.sessionStorage.getItem(CHART_DRAFT_KEY)).toBeNull()
  })

  it("ignores a draft another board left behind", async () => {
    stubPage("edit")
    writeChartDraft({ board: "b1", def: DRAFT })
    render(<ChartBuilderPage boardId="b2" />)

    await screen.findByRole("heading", { level: 1 })
    expect((screen.getByLabelText(/^Title/) as HTMLInputElement).value).toBe("")
    expect(window.sessionStorage.getItem(CHART_DRAFT_KEY)).toBeNull()
  })

  it("draws the preview with the address's parameter values and saves the chart without them", async () => {
    stubPage("edit")
    search = `?params=${encodeURIComponent('{"region":"eu"}')}`
    writeChartDraft({ board: "b1", def: DRAFT })
    render(<ChartBuilderPage boardId="b1" />)

    // The preview request carries them (BI-11) ...
    await waitFor(
      () => expect(bodies.some((b) => b.url.includes("/specs/preview") && b.body.params)).toBe(true),
      { timeout: 5000 }
    )
    const preview = bodies.find((b) => b.url.includes("/specs/preview"))!
    expect(preview.body.params).toEqual({ region: "eu" })

    fireEvent.click(await screen.findByRole("button", { name: "Create chart" }))

    // ... and the chart that is written does not: they belong to the
    // dashboard, not to the chart.
    await waitFor(() => expect(bodies.some((b) => b.url.includes("/api/dashboard/specs") && !b.url.includes("preview"))).toBe(true))
    const save = bodies.find((b) => b.url.includes("/api/dashboard/specs") && !b.url.includes("preview"))!
    expect(save.method).toBe("POST")
    expect("params" in save.body).toBe(false)
    expect(JSON.stringify(save.body)).not.toContain("eu")
    // Saved from the page, the board is reloaded by going back to it.
    await waitFor(() => expect(pushed).toEqual(["/dashboards/b1"]))
  })
})

describe("the draft the assistant made (WEB-7 D6)", () => {
  it("marks the chat's step saved once the chart is written, and never before", async () => {
    stubPage("edit")
    writeChartDraft({ board: "b1", def: DRAFT, complete: { messageIndex: 1, stepIndex: 2 } })
    render(<ChartBuilderPage boardId="b1" />)

    await waitFor(() =>
      expect((screen.getByLabelText(/^Title/) as HTMLInputElement).value).toBe(
        "Visitors by province"
      )
    )
    // Nothing is claimed while the page is still a draft.
    expect(completed).toEqual([])

    fireEvent.click(screen.getByRole("button", { name: "Create chart" }))

    await waitFor(() => expect(pushed).toEqual(["/dashboards/b1"]))
    expect(completed).toEqual([
      {
        messageIndex: 1,
        stepIndex: 2,
        result: {
          created: true,
          title: "Visitors by province",
          kind: "bar",
          mart: "mart_visits",
          board: "b1",
          via: "builder",
        },
      },
    ])
  })
})

describe("the built-in dashboard (WEB-7 D6)", () => {
  it("reads its level from the starting point, since the access route refuses `default`", async () => {
    const getAccess = spyOn(dashboardService, "getAccess")
    spyOn(dashboardService, "listBoards").mockResolvedValue([{ id: "default", name: "Main" }])
    spyOn(dashboardService, "getChart").mockResolvedValue(null)
    permissions = ["dashboard:write"]
    render(<ChartBuilderPage boardId="default" />)

    expect(await screen.findByRole("heading", { level: 1 })).toHaveProperty(
      "textContent",
      "New chart"
    )
    expect(getAccess).not.toHaveBeenCalled()
  })

  it("shows You don't have access on the built-in board without a write grant", async () => {
    const getAccess = spyOn(dashboardService, "getAccess")
    spyOn(dashboardService, "listBoards").mockResolvedValue([{ id: "default", name: "Main" }])
    spyOn(dashboardService, "getChart").mockResolvedValue(null)
    permissions = ["dashboard:read"]
    render(<ChartBuilderPage boardId="default" />)

    expect(await screen.findByText("You don't have access")).toBeDefined()
    expect(getAccess).not.toHaveBeenCalled()
  })
})
