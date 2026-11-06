// The sections need `next/navigation` (the URL's `?section=` and the
// session provider); stub it before any import resolves (`mock.module` is hoisted).
let search = new URLSearchParams()
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/settings",
  useSearchParams: () => search,
  useRouter: () => ({ push: () => {}, replace: () => {}, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }),
}))

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { SettingsPage } from "./settings-page"

afterEach(() => {
  cleanup()
  mock.restore()
  search = new URLSearchParams()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** Records every path the page read, and answers each with what it needs to render. */
function stubApi() {
  const paths: string[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const path = String(input)
    paths.push(path)
    if (path.includes("/api/auth/me"))
      return json({ id: "u1", name: "Admin", email: null, roles: ["Admin"], permissions: ["settings:write", "governance:write"], tenants: [] })
    if (path.includes("/api/auth/providers")) return json({ oidc: false })
    if (path.includes("/api/governance/tag-keys")) return json([])
    if (path.includes("/api/maps")) return json({ maps: [] })
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
  return paths
}

function renderPage() {
  return render(
    <AuthProvider>
      <SettingsPage />
    </AuthProvider>
  )
}

describe("Settings sections (WEB-3 S3-D6)", () => {
  it("offers the six sections and opens Reporting on the bare address", () => {
    stubApi()
    renderPage()
    for (const name of ["Reporting", "Remembered results", "Query limits", "Sign-in", "Tag keys", "Maps"]) {
      expect(screen.getByRole("tab", { name })).toBeTruthy()
    }
    expect(screen.getByRole("tab", { name: "Reporting" }).getAttribute("aria-selected")).toBe("true")
  })

  it("opens the section the URL names, and loads only that section", async () => {
    search = new URLSearchParams("section=maps")
    const paths = stubApi()
    renderPage()
    expect(screen.getByRole("tab", { name: "Maps" }).getAttribute("aria-selected")).toBe("true")
    expect(await screen.findByText("No map uploaded yet")).toBeTruthy()
    expect(paths.some((p) => p.includes("/api/governance/tag-keys"))).toBe(false)
    expect(paths.some((p) => p.includes("/api/settings/reporting"))).toBe(false)
  })

  it("writes the opened section into the URL", async () => {
    stubApi()
    const replace = spyOn(window.history, "replaceState")
    renderPage()
    fireEvent.click(screen.getByRole("tab", { name: "Tag keys" }))
    expect(await screen.findByText("No tag keys")).toBeTruthy()
    expect(replace).toHaveBeenCalledWith(null, "", "/admin/settings?section=tag-keys")
  })
})
