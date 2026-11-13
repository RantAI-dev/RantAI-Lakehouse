// QA LH-BI-11-05, LH-BI-11-06 (BI-52): a parameter value in the address that
// does not fit no longer replaces the whole dashboard with an error. The board
// is drawn with the value in force, the refusal is shown on its control, and
// the value is taken out of the address. Renders the real page, like
// `dashboard-comments-param.test.tsx`.
const replaced: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/b_1",
  // `?p={"target":"abc"}`, percent-encoded as a browser sends it.
  useSearchParams: () => new URLSearchParams("p=%7B%22target%22%3A%22abc%22%7D"),
  useRouter: () => ({
    push: () => {},
    replace: (url: string) => { replaced.push(url) },
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { DashboardPage } from "./dashboard-page"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
  replaced.length = 0
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

describe("DashboardPage and a refused parameter value (QA LH-BI-11-05)", () => {
  it("a value in the address that does not fit draws the dashboard, says why on the parameter, and is taken out of the address", async () => {
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(json({ id: "u1", name: "Analyst", email: null, roles: [], permissions: ["dashboard:read"], tenants: [] }))
      }
      if (path.startsWith("/api/dashboard?")) {
        return Promise.resolve(json({
          board: "b_1", boardName: "Quarterly Sales", boards: [{ id: "b_1", name: "Quarterly Sales" }],
          charts: [], kpis: [], filterColumns: [], results: {}, layout: {}, canDownload: true,
          params: [{ param: { id: "p_target", name: "target", label: "Target", type: "number", default: 30, min: 0, max: 50 }, value: 30 }],
          appliedParams: { target: 30 },
          refusedParams: [{ name: "target", reason: "parameter 'target' must be a number." }],
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

    // The dashboard is drawn: its parameter control is there, with the value in force.
    expect(await screen.findByLabelText("Target")).toBeTruthy()
    expect((screen.getByLabelText("Target") as HTMLInputElement).value).toBe("30")
    // The refusal is on the page, in the alert that belongs to that parameter.
    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe("parameter 'target' must be a number.")
    expect((document.querySelector('[data-param="target"]') as HTMLElement | null)?.contains(alert)).toBe(true)
    // Not the page's own red error banner, which is for a read that failed:
    // the sentence is in exactly one place, the parameter's alert.
    expect(screen.getAllByText("parameter 'target' must be a number.")).toHaveLength(1)
    expect(document.querySelector('[class~="border-destructive/40"]')).toBeNull()
    // And the value leaves the address.
    await waitFor(() => expect(replaced.length).toBeGreaterThan(0))
    expect(replaced.every((url) => !url.includes("p="))).toBe(true)
  })
})
