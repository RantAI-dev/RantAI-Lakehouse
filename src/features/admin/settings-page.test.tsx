// The sections need `next/navigation` (the URL's `?section=` and the
// session provider); stub it before any import resolves (`mock.module` is hoisted).
let search = new URLSearchParams()
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/settings",
  useSearchParams: () => search,
  useRouter: () => ({ push: () => {}, replace: () => {}, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
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
function stubApi(permissions = ["settings:write", "governance:write"]) {
  const paths: string[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const path = String(input)
    paths.push(path)
    if (path.includes("/api/auth/me"))
      return json({ id: "u1", name: "Admin", email: null, roles: ["Admin"], permissions, tenants: [] })
    if (path.includes("/api/auth/providers")) return json({ oidc: false })
    if (path.includes("/api/governance/tag-keys")) return json([])
    if (path.includes("/api/maps")) return json({ maps: [] })
    if (path.includes("/api/settings/deployment")) return json({ features: [] })
    if (path.includes("/api/settings/email"))
      return json({
        host: { value: null, source: "default" },
        port: { value: 587, source: "default" },
        security: { value: "starttls", source: "default" },
        user: { value: null, source: "default" },
        password: { set: false, source: "default" },
        from: { value: null, source: "default" },
        consolePublicUrl: { value: null, source: "default" },
      })
    if (path.includes("/api/settings/sign-in"))
      return json({
        maxFailures: { value: 5, source: "default" },
        failureWindowSecs: { value: 900, source: "default" },
        lockoutSecs: { value: 300, source: "default" },
        retentionDays: { value: 30, source: "default" },
        embedMaxLifetimeSecs: { value: 86400, source: "default" },
      })
    if (path.includes("/api/settings/assistant"))
      return json({
        url: { value: "https://api.example.com/v1", source: "default" },
        model: { value: "example-model", source: "default" },
        key: { set: false, source: "default" },
      })
    if (path.includes("/api/settings/sso"))
      return json({
        issuer: { value: null, source: "default" },
        clientId: { value: null, source: "default" },
        clientSecret: { set: false, source: "default" },
        providerName: { value: "default", source: "default" },
        groupsClaim: { value: "groups", source: "default" },
        roleMap: { value: [], source: "default" },
        jitProvisioning: { value: false, source: "default" },
        canEdit: true,
      })
    if (path.includes("/api/identity/roles")) return json([])
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
  it("offers the six sections and the three administrator ones, and opens Deployment for an administrator", async () => {
    stubApi()
    renderPage()
    for (const name of [
      "Reporting",
      "Remembered results",
      "Query limits",
      "Sign-in",
      "Tag keys",
      "Maps",
      "Deployment",
      "Email",
      "Assistant",
    ]) {
      expect(await screen.findByRole("tab", { name })).toBeTruthy()
    }
    // `ADM-6`, `ADM-7`: the deployment's own sections come first, so the
    // bare address opens the Deployment one.
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Deployment" }).getAttribute("aria-selected")).toBe("true")
    )
  })

  it("shows only the six sections to a caller without settings:write", async () => {
    stubApi(["dashboard:read"])
    renderPage()
    for (const name of ["Reporting", "Remembered results", "Query limits", "Sign-in", "Tag keys", "Maps"]) {
      expect(await screen.findByRole("tab", { name })).toBeTruthy()
    }
    await waitFor(() => expect(screen.queryByRole("tab", { name: "Deployment" })).toBeNull())
    expect(screen.queryByRole("tab", { name: "Email" })).toBeNull()
    expect(screen.queryByRole("tab", { name: "Assistant" })).toBeNull()
  })

  it("opens Reporting on the bare address for everybody else", async () => {
    stubApi(["dashboard:read"])
    renderPage()
    await screen.findByRole("tab", { name: "Reporting" })
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Reporting" }).getAttribute("aria-selected")).toBe("true")
    )
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
