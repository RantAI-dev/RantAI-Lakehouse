mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/b_1",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { toast } from "sonner"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import * as realEchart from "./echart"

// A tile the date grouping does not change needs rows, and a chart needs a
// canvas; the mark does not. The real module is put back for the files after
// this one, as chart-builder.test.tsx does for its own mock.
mock.module("./echart", () => ({ EChart: () => <div data-testid="chart" /> }))
afterAll(() => {
  mock.module("./echart", () => realEchart)
})

import { DashboardPage } from "./dashboard-page"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

describe("DashboardPage per-role rendering without CopilotProvider (ADM-13 review round 1, S6)", () => {
  it("renders for a dashboard:read-only user without CopilotProvider, using useCopilot fallback", async () => {
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(
          json({
            id: "u_viewer",
            name: "Dashboard Viewer",
            email: null,
            roles: ["Dashboard Viewer"],
            permissions: ["dashboard:read"],
            tenants: [],
          }),
        )
      }
      if (path.startsWith("/api/dashboard?")) {
        return Promise.resolve(
          json({
            board: "b_1",
            boardName: "Quarterly Sales",
            boards: [{ id: "b_1", name: "Quarterly Sales" }],
            charts: [],
            kpis: [],
            filterColumns: [],
            results: {},
            layout: {},
            canDownload: true,
          }),
        )
      }
      if (path.includes("/api/dashboard/folders")) {
        return Promise.resolve(json([]))
      }
      if (path.includes("/api/dashboard/shortcuts")) {
        return Promise.resolve(json({ favourites: [], recent: [] }))
      }
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)

    // Render under AuthProvider and TableProviders, but NOT CopilotProvider
    // (simulating AppFrame when user lacks assistant permissions).
    render(
      <AuthProvider>
        <TableProviders>
          <DashboardPage boardId="b_1" />
        </TableProviders>
      </AuthProvider>,
    )

    // The dashboard header and switcher render with the board name
    expect(await screen.findByText("Quarterly Sales")).toBeTruthy()

    // Assert that the page rendered in view mode without errors
    expect(screen.queryByRole("alert")).toBeNull()
  })
})

// QA LH-BI-9-05 (BI-50): the mark a chart the dashboard's date grouping does
// not change carries is drawn only while the control applied a grain.
describe("DashboardPage date grouping mark (QA LH-BI-9-05)", () => {
  const sentence = "Group dates by does not apply: this chart's dimension is not a date."
  const grained = {
    id: "c_line", title: "Visitors", kind: "line", mart: "mart_x", x: "day", y: "visitors",
    source: "ui", span: 1,
    def: { title: "Visitors", kind: "line", mart: "mart_x", dimension: "day", measures: ["visitors"], grain: "day" },
  }
  const byCity = {
    id: "c_city", title: "By city", kind: "bar", mart: "mart_x", x: "city", y: "visitors",
    source: "ui", span: 1,
    def: { title: "By city", kind: "bar", mart: "mart_x", dimension: "city", measures: ["visitors"] },
  }

  function mount(appliedGrain: string | null) {
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(json({
          id: "u_viewer", name: "Dashboard Viewer", email: null,
          roles: ["Dashboard Viewer"], permissions: ["dashboard:read"], tenants: [],
        }))
      }
      if (path.startsWith("/api/dashboard?")) {
        return Promise.resolve(json({
          board: "b_1", boardName: "Quarterly Sales", boards: [{ id: "b_1", name: "Quarterly Sales" }],
          charts: [grained, byCity], kpis: [], filterColumns: [], layout: { c_line: { x: 0, y: 0, w: 6, h: 4 }, c_city: { x: 0, y: 4, w: 6, h: 4 } },
          grain: "", appliedGrain, canDownload: true,
          results: {
            c_line: { columns: ["day", "visitors"], rows: [{ day: "2026-01-01", visitors: 1 }], grain: appliedGrain ?? "day" },
            c_city: { columns: ["city", "visitors"], rows: [{ city: "Bali", visitors: 2 }], grainNotApplied: "not_a_date" },
          },
        }))
      }
      if (path.includes("/api/dashboard/folders")) return Promise.resolve(json([]))
      if (path.includes("/api/dashboard/shortcuts")) return Promise.resolve(json({ favourites: [], recent: [] }))
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)
    render(
      <AuthProvider>
        <TableProviders>
          <DashboardPage boardId="b_1" />
        </TableProviders>
      </AuthProvider>,
    )
  }

  it("the mark is drawn only while the dashboard applied a grain", async () => {
    mount("month")
    expect(await screen.findByLabelText(sentence)).toBeTruthy()

    cleanup()
    mount(null)
    expect(await screen.findByText("By city")).toBeTruthy()
    expect(screen.queryByLabelText(sentence)).toBeNull()
  })
})

// `BI-48` (QA `LH-BI-6-40`): a chart tile's menu copies the chart — a new chart
// on the same dashboard from the stored definition, titled "<title> (copy)",
// placed directly below the original.
describe("DashboardPage tile menu: Duplicate (QA LH-BI-6-40)", () => {
  const chart = {
    id: "c_1", title: "Visitors", kind: "bar", mart: "mart_x", x: "provinsi", y: "visitors",
    source: "ui", span: 1,
    def: { title: "Visitors", kind: "bar", mart: "mart_x", dimension: "provinsi", measures: ["visitors"] },
  }

  it("posts the chart's definition with a (copy) title and the board, then saves a layout that holds the new id", async () => {
    const writes: { url: string; method: string; body: Record<string, unknown> }[] = []
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input)
      const method = init?.method ?? "GET"
      const body = init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : {}
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(json({ id: "u_editor", name: "Editor", email: null, roles: ["Editor"], permissions: ["dashboard:read", "dashboard:write"], tenants: [] }))
      }
      if (path.startsWith("/api/dashboard?") && method === "GET") {
        return Promise.resolve(json({
          board: "b_1", boardName: "Quarterly Sales", boards: [{ id: "b_1", name: "Quarterly Sales" }],
          charts: [chart], kpis: [], filterColumns: [], results: {},
          layout: { c_1: { x: 0, y: 0, w: 6, h: 4 } }, canDownload: true,
        }))
      }
      if (path.startsWith("/api/dashboard/specs") && method === "POST") {
        writes.push({ url: path, method, body })
        return Promise.resolve(json({ ok: true, chart: { id: "c_copy" } }))
      }
      if (path.startsWith("/api/dashboard/boards") && method === "PUT") {
        writes.push({ url: path, method, body })
        return Promise.resolve(json({ ok: true }))
      }
      if (path.includes("/api/dashboard/folders")) return Promise.resolve(json([]))
      if (path.includes("/api/dashboard/shortcuts")) return Promise.resolve(json({ favourites: [], recent: [] }))
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)

    render(
      <AuthProvider>
        <TableProviders>
          <DashboardPage boardId="b_1" />
        </TableProviders>
      </AuthProvider>,
    )

    fireEvent.click(await screen.findByLabelText("Actions for Visitors"))
    fireEvent.click(await screen.findByText("Duplicate"))
    await waitFor(() => expect(writes.map((w) => w.method)).toEqual(["POST", "PUT"]))

    expect(writes[0].body).toMatchObject({
      kind: "bar", mart: "mart_x", dimension: "provinsi", measures: ["visitors"],
      title: "Visitors (copy)", board: "b_1",
    })
    expect((writes[0].body as { id?: string }).id).toBeUndefined()
    const layout = writes[1].body.layout as Record<string, { x: number; y: number; w: number; h: number }>
    expect(layout.c_copy).toEqual({ x: 0, y: 4, w: 6, h: 4 })
  })

  // `BI-48` review round 1, R6: the chart is one write and its place another.
  // When the place cannot be saved the copy still exists, so the page reloads
  // and the toast is not "The chart was not duplicated".
  it("a layout save that fails still reloads and does not say the chart was not duplicated", async () => {
    const posts: Record<string, unknown>[] = []
    const toasts: { title: string; description?: string }[] = []
    let gets = 0
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input)
      const method = init?.method ?? "GET"
      const body = init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : {}
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(json({ id: "u_editor", name: "Editor", email: null, roles: ["Editor"], permissions: ["dashboard:read", "dashboard:write"], tenants: [] }))
      }
      if (path.startsWith("/api/dashboard?") && method === "GET") {
        gets += 1
        // The reload after the failed layout save is the one that brings the
        // copy in; nothing else was clicked between.
        const charts = gets > 1 ? [chart, { ...chart, id: "c_copy", title: "Visitors (copy)" }] : [chart]
        return Promise.resolve(json({
          board: "b_1", boardName: "Quarterly Sales", boards: [{ id: "b_1", name: "Quarterly Sales" }],
          charts, kpis: [], filterColumns: [], results: {},
          layout: { c_1: { x: 0, y: 0, w: 6, h: 4 } }, canDownload: true,
        }))
      }
      if (path.startsWith("/api/dashboard/specs") && method === "POST") {
        posts.push(body)
        return Promise.resolve(json({ ok: true, chart: { id: "c_copy" } }))
      }
      if (path.startsWith("/api/dashboard/boards") && method === "PUT") {
        return Promise.resolve(json({ error: "The layout could not be saved." }, 500))
      }
      if (path.includes("/api/dashboard/folders")) return Promise.resolve(json([]))
      if (path.includes("/api/dashboard/shortcuts")) return Promise.resolve(json({ favourites: [], recent: [] }))
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)
    spyOn(toast, "error").mockImplementation((title, opts) => {
      toasts.push({ title: String(title), description: opts?.description ? String(opts.description) : undefined })
      return 0
    })

    render(
      <AuthProvider>
        <TableProviders>
          <DashboardPage boardId="b_1" />
        </TableProviders>
      </AuthProvider>,
    )

    fireEvent.click(await screen.findByLabelText("Actions for Visitors"))
    fireEvent.click(await screen.findByText("Duplicate"))
    // The copy is on screen after the reload, from one POST: no second click.
    expect(await screen.findByText("Visitors (copy)")).toBeTruthy()
    expect(posts).toHaveLength(1)
    expect(gets).toBeGreaterThan(1)
    const titles = toasts.map((t) => t.title)
    expect(titles).not.toContain("The chart was not duplicated")
    expect(titles).toContain("The copy was made, but its place was not saved")
  })
})
