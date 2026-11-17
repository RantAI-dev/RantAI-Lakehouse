// QA LH-BI-2-05 (BI-54): without `metric:write` the metrics page is a list to
// read — no "New metric", no editor, no "New segment" and no "Edit" — with one
// sentence saying who may change them. The permissions come from the real
// session check, as in `dashboard-page-role.test.tsx`.
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/metrics",
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

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { MetricsPage } from "./metrics-page"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const METRIC = {
  id: "m1", name: "coba_metrik", label: "Coba metrik", description: "A seeded metric.",
  sourceKind: "mart", sourceId: "mart_visits", formula: "sum(visitors)", filters: [],
  createdBy: "ui", usedBy: 2, sourceTitle: "Visits",
}
const SEGMENT = {
  id: "s1", name: "domestic", label: "Domestic visitors", sourceKind: "mart",
  sourceId: "mart_visits", filters: [], createdBy: "ui",
}

/** The page over the session of a person holding `permissions`. */
function mount(permissions: string[]) {
  spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
    const path = String(input)
    if (path.includes("/api/auth/me")) {
      return Promise.resolve(json({
        id: "u_qa", name: "QA person", email: null, roles: [], permissions, tenants: [],
      }))
    }
    if (path.startsWith("/api/dashboard/metrics")) return Promise.resolve(json({ metrics: [METRIC] }))
    if (path.startsWith("/api/dashboard/segments")) return Promise.resolve(json({ segments: [SEGMENT] }))
    if (path.includes("/api/dashboard/sources")) return Promise.resolve(json({ sources: [] }))
    if (path.includes("/api/dashboard/fields")) return Promise.resolve(json({ marts: [], dimensions: [], measures: [], columns: [] }))
    return Promise.resolve(json({}))
  }) as unknown as typeof fetch)
  render(
    <AuthProvider>
      <TableProviders>
        <MetricsPage />
      </TableProviders>
    </AuthProvider>,
  )
}

describe("MetricsPage and metric:write (QA LH-BI-2-05)", () => {
  it("a person without metric:write reads the list and is offered nothing to define or change", async () => {
    mount(["dashboard:read"])
    expect(await screen.findByText("Coba metrik")).toBeTruthy()
    expect(screen.queryByText("New metric")).toBeNull()
    expect(screen.getByText("You can read metrics and segments. Defining or changing one needs the metric:write permission.")).toBeTruthy()

    // A row opens no editor.
    fireEvent.click(screen.getByText("Coba metrik"))
    expect(screen.queryByRole("dialog")).toBeNull()

    // The Segments dialog lists them, with nothing to define or change.
    fireEvent.click(screen.getByText("Segments"))
    expect(await screen.findByText("Domestic visitors")).toBeTruthy()
    expect(screen.queryByText("New segment")).toBeNull()
    expect(screen.queryByText("Edit")).toBeNull()
  })

  it("a person with metric:write is offered New metric, New segment and Edit", async () => {
    mount(["dashboard:read", "metric:write"])
    expect(await screen.findByText("Coba metrik")).toBeTruthy()
    expect(screen.getByText("New metric")).toBeTruthy()
    expect(screen.queryByText("You can read metrics and segments. Defining or changing one needs the metric:write permission.")).toBeNull()

    fireEvent.click(screen.getByText("Segments"))
    expect(await screen.findByText("Domestic visitors")).toBeTruthy()
    expect(screen.getByText("New segment")).toBeTruthy()
    expect(screen.getByText("Edit")).toBeTruthy()
  })
})
