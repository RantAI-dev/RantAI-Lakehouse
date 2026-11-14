mock.module("next/navigation", () => ({
  usePathname: () => "/catalog",
  useSearchParams: () => new URLSearchParams(""),
  useRouter: () => ({ push: () => {}, replace: () => {}, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }),
}))
mock.module("next/image", () => ({ default: () => null }))

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { SidebarProvider } from "@/components/ui/sidebar"
import { act } from "@testing-library/react"
import { forgetRow, rememberRow } from "@/features/dashboards/dashboards-row"
import { readLastBoard, rememberLastBoard } from "@/features/dashboards/last-board"
import { AppSidebar } from "./app-sidebar"

const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } })

afterEach(() => {
  cleanup()
  window.localStorage.clear()
  mock.restore()
})

function renderSidebar(open: boolean) {
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    if (String(input).includes("/api/auth/me")) {
      return json({ id: "u1", name: "Ana", email: "ana@example.invalid", roles: ["R"], permissions: ["catalog:read", "query:read", "dashboard:read"], tenants: [] })
    }
    return json({})
  }) as unknown as typeof fetch)
  return render(
    <AuthProvider>
      <SidebarProvider open={open} onOpenChange={() => {}}>
        <AppSidebar />
      </SidebarProvider>
    </AuthProvider>,
  )
}

// Owner 2026-11-14 ("aku kurang suka animasi saat collapse sidebar"): a section is one row in both
// states, so nothing is swapped as the sidebar moves; its pages fold away instead of disappearing.
describe("AppSidebar, one row per section in both states", () => {
  it("expanded: the section's header opens its pages in place, and they are reachable", async () => {
    renderSidebar(true)
    const data = await screen.findByRole("button", { name: "Data" })
    expect(data.getAttribute("aria-expanded")).toBe("true")
    const catalog = screen.getByRole("link", { name: "Catalog" })
    expect(catalog.closest("[inert]")).toBeNull()
  })

  it("collapsed: the same row opens a flyout, and the folded pages take no focus", async () => {
    renderSidebar(false)
    const data = await screen.findByTitle("Data")
    expect(data.tagName).toBe("BUTTON")
    expect(data.getAttribute("aria-expanded")).toBe("false")
    const folded = document.querySelector("[data-slot='sidebar-menu-sub']")?.closest("[inert]")
    expect(folded).not.toBeNull()
    fireEvent.click(data)
    expect(data.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getAllByRole("link", { name: "Catalog" }).some((a) => a.closest("[inert]") === null)).toBe(true)
  })
})

// Owner 2026-11-14: the Dashboards row is a split button group; the right part is the whole
// touch target for Browse, its own link and tab stop, with the label for assistive tech.
describe("AppSidebar, the Dashboards split row", () => {
  it("has the label link and a separate full-height Browse link, joined by a separator", async () => {
    renderSidebar(true)
    const dashboards = await screen.findByRole("link", { name: "Dashboards" })
    const browse = screen.getByRole("link", { name: "Browse all dashboards" })
    expect(browse.getAttribute("href")).toBe("/dashboards/browse")
    expect(dashboards).not.toBe(browse)
    expect(browse.className).toContain("size-8")
    expect(dashboards.closest("li")).toBe(browse.closest("li"))
    expect(browse.previousElementSibling?.className).toContain("w-px")
  })
})

// BI-61 T10/T11 (owner QA 2026-11-15): the row is named after the last dashboard or report
// opened, in its own memory that Browse clears and the last board does not share.
describe("AppSidebar, the Dashboards row names what it opens", () => {
  const row = () => document.querySelector("a[href^='/dashboards']:not([aria-label='Browse all dashboards'])") as HTMLAnchorElement

  it("says Dashboards and goes to Browse when nothing is remembered", async () => {
    renderSidebar(true)
    await screen.findByRole("link", { name: "Dashboards" })
    expect(row().getAttribute("href")).toBe("/dashboards/browse")
  })

  it("is named after the remembered dashboard and goes to it", async () => {
    rememberRow("dashboard", "b1", "Revenue")
    renderSidebar(true)
    const link = await screen.findByRole("link", { name: "Dashboard: Revenue" })
    expect(link.textContent).toBe("Revenue")
    expect(link.getAttribute("href")).toBe("/dashboards/b1")
  })

  it("a report sets the name, the report href and the report icon; a dashboard then replaces it", async () => {
    rememberRow("report", "r1", "Q3 review")
    renderSidebar(true)
    const link = await screen.findByRole("link", { name: "Report: Q3 review" })
    expect(link.getAttribute("href")).toBe("/dashboards/reports/r1")
    const reportIcon = link.querySelector("svg")?.getAttribute("class")
    expect(reportIcon).toContain("lucide-file-text")
    act(() => rememberRow("dashboard", "b2", "Ops"))
    const dash = await screen.findByRole("link", { name: "Dashboard: Ops" })
    expect(dash.getAttribute("href")).toBe("/dashboards/b2")
    expect(dash.querySelector("svg")?.getAttribute("class")).toContain("lucide-layout-grid")
  })

  it("Browse clears the row but not the last board", async () => {
    rememberLastBoard("b1")
    rememberRow("dashboard", "b1", "Revenue")
    renderSidebar(true)
    await screen.findByRole("link", { name: "Dashboard: Revenue" })
    act(() => forgetRow())
    expect((await screen.findByRole("link", { name: "Dashboards" })).getAttribute("href")).toBe("/dashboards/browse")
    expect(readLastBoard()).toBe("b1")
  })

  it("deleting the remembered item clears the row; deleting another does not", async () => {
    rememberRow("report", "r1", "Q3 review")
    renderSidebar(true)
    await screen.findByRole("link", { name: "Report: Q3 review" })
    act(() => forgetRow({ kind: "dashboard", id: "r1" }))
    act(() => forgetRow({ kind: "report", id: "other" }))
    await screen.findByRole("link", { name: "Report: Q3 review" })
    act(() => forgetRow({ kind: "report", id: "r1" }))
    await screen.findByRole("link", { name: "Dashboards" })
  })

  it("a rename updates the label at once", async () => {
    rememberRow("dashboard", "b2", "Ops")
    renderSidebar(true)
    await screen.findByRole("link", { name: "Dashboard: Ops" })
    act(() => rememberRow("dashboard", "b2", "Operations"))
    await screen.findByRole("link", { name: "Dashboard: Operations" })
  })

  it("ignores a malformed stored value", async () => {
    window.localStorage.setItem("dashboards:menu-row", "{nope")
    renderSidebar(true)
    expect((await screen.findByRole("link", { name: "Dashboards" })).getAttribute("href")).toBe("/dashboards/browse")
  })
})
