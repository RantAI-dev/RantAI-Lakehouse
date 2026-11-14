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
import { AppSidebar } from "./app-sidebar"

const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } })

afterEach(() => {
  cleanup()
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
