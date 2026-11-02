// BI-19 owner QA 2026-10-28, QA-1: a board the caller cannot open shows the error and nothing
// that acts on the board. Renders the real page, like `dashboard-stop.test.tsx`; `fetch` is the
// only door to the network.
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
import { CopilotProvider } from "@/features/copilot/use-copilot"
import { DashboardPage } from "./dashboard-page"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

describe("DashboardPage on a board that is not found (BI-19 QA-1)", () => {
  it("shows the error and a link back, with no star, menu, comments or empty-chart state", async () => {
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(json({ id: "u1", name: "Analyst", email: null, roles: ["Analyst"], permissions: [], tenants: [] }))
      }
      if (path.startsWith("/api/dashboard?")) return Promise.resolve(json({ error: "dashboard not found" }, 404))
      if (path.includes("/api/ai/sessions")) return Promise.resolve(json({ sessions: [] }))
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)

    render(
      <AuthProvider>
        <TableProviders>
          <CopilotProvider>
            <DashboardPage boardId="b_1" />
          </CopilotProvider>
        </TableProviders>
      </AuthProvider>,
    )

    expect((await screen.findByRole("alert")).textContent).toMatch(/not found/i)
    expect(screen.getByRole("link", { name: "All dashboards" }).getAttribute("href")).toBe("/dashboards/browse")
    expect(screen.queryByRole("button", { name: /comments/i })).toBeNull()
    expect(screen.queryByRole("button", { name: /favourite|favorite/i })).toBeNull()
    expect(screen.queryByText("No charts yet")).toBeNull()
  })
})
