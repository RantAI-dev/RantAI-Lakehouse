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

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
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
