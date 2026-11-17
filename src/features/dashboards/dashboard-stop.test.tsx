// BI-5 (plan D5, feature decision 4): Stop on a dashboard that is still loading. This renders the real page. Its two providers (`AuthProvider`, `CopilotProvider`) and
// its child panels reach the network only through `fetch` and the service
// objects, which are spied here, so nothing leaves the test.
// `next/navigation` is stubbed before any import resolves (`mock.module` is
// hoisted), and `TableProviders` supplies the URL state the results table
// needs, as the root layout does.
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/default",
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
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { CopilotProvider } from "@/features/copilot/use-copilot"
import { queryService } from "@/services"
import { DashboardPage } from "./dashboard-page"


afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

describe("DashboardPage Stop (BI-5)", () => {
  it("shows Stop while the dashboard loads; it asks the server to cancel the run named in X-Run-Id, aborts the request, and says Stopped", async () => {
    let load: { runId: string | null; signal?: AbortSignal } | null = null
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(json({ id: "u1", name: "Analyst", email: null, roles: ["Analyst"], permissions: [], tenants: [] }))
      }
      if (path.startsWith("/api/dashboard?")) {
        load = { runId: new Headers(init?.headers).get("X-Run-Id"), signal: init?.signal ?? undefined }
        return new Promise((_resolve, reject) => init?.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError"))))
      }
      if (path.includes("/api/ai/sessions")) return Promise.resolve(json({ sessions: [] }))
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)
    const cancel = spyOn(queryService, "cancel").mockResolvedValue({ stopped: 3 })

    render(
      <AuthProvider>
        <TableProviders>
          <CopilotProvider>
            <DashboardPage boardId="default" />
          </CopilotProvider>
        </TableProviders>
      </AuthProvider>,
    )

    const stop = await screen.findByRole("button", { name: "Stop" })
    await waitFor(() => expect(load).not.toBeNull())
    const sent = load as unknown as { runId: string | null; signal?: AbortSignal }
    expect(sent.runId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/)
    fireEvent.click(stop)
    expect(cancel).toHaveBeenCalledTimes(1)
    expect(cancel.mock.calls[0][0]).toBe(sent.runId as string)
    expect(sent.signal?.aborted).toBe(true)
    expect(await screen.findByText(/^Stopped\./)).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull()
    expect(screen.queryByText("No charts yet")).toBeNull()
  })
})
