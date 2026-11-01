// BI-19 owner QA 2026-10-28, QA-3: `?comments=1` (the link a bell mention carries) opens the
// Comments dialog once the board is loaded, and closing it removes the parameter while keeping
// the others. Renders the real page, like `dashboard-stop.test.tsx`.
const replaced: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/b_1",
  useSearchParams: () => new URLSearchParams("comments=1&f=x"),
  useRouter: () => ({
    push: () => {},
    replace: (url: string) => { replaced.push(url) },
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
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

describe("DashboardPage ?comments=1 (BI-19 QA-3)", () => {
  it("opens the Comments dialog for a loaded board and drops only that parameter on close", async () => {
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(json({ id: "u1", name: "Analyst", email: null, roles: [], permissions: [], tenants: [] }))
      }
      if (path.startsWith("/api/dashboard?")) return Promise.resolve(json({ charts: [] }))
      if (path.includes("/comments")) return Promise.resolve(json({ comments: [], truncated: false }))
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

    expect(await screen.findByRole("list", { name: "Comments" })).toBeTruthy()
    fireEvent.click(screen.getAllByRole("button", { name: "Close" })[0])
    await waitFor(() => expect(replaced.at(-1)).toBe("/dashboards/b_1?f=x"))
  })
})
