// `adm-121`, decision 1: the console's shell asks `GET /api/branding` once
// for the sidebar, the navbar, the palette and the General settings page,
// and shows the product's name for none of them before the answer. Only
// `fetch` is stubbed; the real `apiFetch`, services and shell run.
mock.module("next/navigation", () => ({
  usePathname: () => "/admin/settings",
  useSearchParams: () => new URLSearchParams(""),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import * as React from "react"
import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { AppFrame } from "@/components/app-shell/app-frame"
import { TooltipProvider } from "@/components/ui/tooltip"
import { AuthProvider } from "@/features/auth/auth-provider"
import { GeneralSettings } from "@/features/admin/general-settings"

const originalFetch = global.fetch

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })

const BRANDING = {
  workspaceName: "Acme Data",
  logo: { type: "image/png", version: "v1" },
  logoDark: null,
  icon: null,
}

/**
 * Serves the session check and the settings read at once, and holds the
 * branding answer until `release` is called, so a test can look at the page
 * while the answer is still on its way.
 */
function stubApi() {
  const urls: string[] = []
  let release: () => void = () => {}
  const held = new Promise<void>((resolve) => {
    release = resolve
  })
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = String(input)
    urls.push(url)
    if (url.includes("/api/auth/me")) {
      return json({
        id: "u1",
        name: "Test User",
        email: "test@rantai.dev",
        roles: ["Admin"],
        permissions: ["settings:write"],
        tenants: [{ id: "t1", name: "Default Tenant", role: "Admin" }],
      })
    }
    if (url.includes("/api/branding")) {
      await held
      return json(BRANDING)
    }
    if (url.includes("/api/settings/general")) {
      return json({
        workspaceName: "Acme Data",
        logo: { set: true, type: "image/png", bytes: 1024 },
        logoDark: { set: false, type: null, bytes: null },
        icon: { set: false, type: null, bytes: null },
        updatedAt: "2026-11-16T09:00:00Z",
      })
    }
    return json({})
  }) as unknown as typeof fetch
  return { urls, release }
}

function renderShell() {
  return render(
    <TooltipProvider>
      <AuthProvider>
        <AppFrame>
          <GeneralSettings />
        </AppFrame>
      </AuthProvider>
    </TooltipProvider>,
  )
}

describe("the console shell reads the workspace's branding once (adm-121)", () => {
  it("makes one GET /api/branding for a page that shows the name in four places", async () => {
    const { urls, release } = stubApi()
    renderShell()
    // The sidebar is drawn once the session answers; the General page's
    // own form is drawn once its settings answer. Both ask for the branding.
    await screen.findByLabelText("Workspace name")
    release()
    await waitFor(() => expect(screen.getAllByText("Acme Data").length).toBeGreaterThan(0))
    // Let any request a second reader would still send go out.
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(urls.filter((url) => url.includes("/api/branding"))).toEqual(["/api/branding"])
  })

  it("shows no product name anywhere before the branding answers", async () => {
    const { release } = stubApi()
    const { container } = renderShell()
    await screen.findByLabelText("Workspace name")
    // The page is drawn and the answer is held: the sidebar's name slot is
    // empty, not "Rantai Lake" for a moment before "Acme Data".
    expect(container.textContent).not.toContain("Rantai Lake")
    expect(container.textContent).not.toContain("Acme Data")
    release()
    await waitFor(() => expect(container.textContent).toContain("Acme Data"))
    expect(container.textContent).not.toContain("Rantai Lake")
  })
})
