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
import { forgetLastBoard, rememberLastBoard } from "@/features/dashboards/last-board"
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

// BI-61 T10 (owner QA 2026-11-15): the row is named after the dashboard it opens.
describe("AppSidebar, the Dashboards row names the board it opens", () => {
  const row = () => document.querySelector("a[href^='/dashboards']:not([aria-label='Browse all dashboards'])") as HTMLAnchorElement

  it("says Dashboards and goes to Browse when no board is remembered", async () => {
    renderSidebar(true)
    await screen.findByRole("link", { name: "Dashboards" })
    expect(row().getAttribute("href")).toBe("/dashboards/browse")
  })

  it("is named after the remembered board and goes to it", async () => {
    rememberLastBoard("b1", "Revenue")
    renderSidebar(true)
    const link = await screen.findByRole("link", { name: "Dashboard: Revenue" })
    expect(link.textContent).toBe("Revenue")
    expect(link.getAttribute("href")).toBe("/dashboards/b1")
  })

  it("keeps Dashboards for an old id-only value, but still goes to the board", async () => {
    window.localStorage.setItem("dashboards:last-board", "b1")
    renderSidebar(true)
    const link = await screen.findByRole("link", { name: "Dashboards" })
    expect(link.getAttribute("href")).toBe("/dashboards/b1")
  })

  it("forgetting, which is what opening Browse does, brings Dashboards and the Browse href back", async () => {
    rememberLastBoard("b1", "Revenue")
    renderSidebar(true)
    await screen.findByRole("link", { name: "Dashboard: Revenue" })
    act(() => forgetLastBoard())
    expect((await screen.findByRole("link", { name: "Dashboards" })).getAttribute("href")).toBe("/dashboards/browse")
  })

  it("opening another board, or renaming it, updates the label at once", async () => {
    rememberLastBoard("b1", "Revenue")
    renderSidebar(true)
    await screen.findByRole("link", { name: "Dashboard: Revenue" })
    act(() => rememberLastBoard("b2", "Ops"))
    expect((await screen.findByRole("link", { name: "Dashboard: Ops" })).getAttribute("href")).toBe("/dashboards/b2")
    act(() => rememberLastBoard("b2", "Operations"))
    await screen.findByRole("link", { name: "Dashboard: Operations" })
    // a name belonging to another board is never shown
    act(() => rememberLastBoard("b3"))
    expect((await screen.findByRole("link", { name: "Dashboards" })).getAttribute("href")).toBe("/dashboards/b3")
  })
})
