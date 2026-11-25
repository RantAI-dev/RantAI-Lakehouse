import * as React from "react"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import {
  PRODUCT_LOGO_DARK,
  PRODUCT_LOGO_LIGHT,
  PRODUCT_NAME,
} from "@/lib/product-branding"

// `AppSidebar` reads the route through `next/navigation`, which needs a
// Next.js AppRouter context that a bare happy-dom environment does not
// have, so the module is stubbed at the boundary (the same pattern as
// `per-role-navigation.test.tsx`).
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards",
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

import { AppSidebar } from "@/components/app-shell/app-sidebar"
import { SidebarProvider } from "@/components/ui/sidebar"
import { TooltipProvider } from "@/components/ui/tooltip"
import { AuthProvider } from "@/features/auth/auth-provider"

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })

const originalFetch = global.fetch

/** The session the sidebar's chrome needs; the branding is what it shows. */
const SESSION = {
  id: "u1",
  name: "Test User",
  email: "test@rantai.dev",
  roles: ["Viewer"],
  permissions: ["dashboard:read"],
  tenants: [{ id: "t1", name: "Default Tenant", role: "Viewer" }],
}

/**
 * Serves the session check and the public branding route. The branding is
 * what the header shows; the session check only keeps the console chrome
 * rendered.
 *
 * With `hold`, the branding answer waits for `release`, so a test can look
 * at the header while the answer is still on its way.
 */
function stubApi(
  branding: unknown,
  options: { hold?: boolean; permissions?: string[]; deployment?: unknown } = {},
): { requested: string[]; release: () => void } {
  const requested: string[] = []
  let release: () => void = () => {}
  const held = new Promise<void>((resolve) => {
    release = resolve
  })
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = String(input)
    requested.push(url)
    if (url.includes("/api/auth/me")) {
      return json({ ...SESSION, permissions: options.permissions ?? SESSION.permissions })
    }
    if (url.includes("/api/settings/deployment")) {
      return json(options.deployment ?? { features: [] })
    }
    if (url.includes("/api/branding")) {
      if (options.hold) await held
      return json(branding)
    }
    return json({})
  }) as unknown as typeof fetch
  return { requested, release }
}

function renderSidebar() {
  return render(
    <TooltipProvider>
      <SidebarProvider>
        <AuthProvider>
          <AppSidebar />
        </AuthProvider>
      </SidebarProvider>
    </TooltipProvider>,
  )
}

/**
 * The header's two pictures, in the order they are drawn: the light one
 * first, the dark one second.
 */
function pictures(container: HTMLElement): HTMLImageElement[] {
  return Array.from(container.querySelectorAll("img"))
}

beforeEach(() => {
  // happy-dom starts on `about:blank`: a relative URL has no base to
  // resolve against, so a picture with a relative source counts as broken
  // and the branding falls back to the product's own file. The pictures
  // must resolve the way they do in a browser (the same setup as
  // `public-dashboard.test.tsx`).
  ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(
    "http://localhost/",
  )
})

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

describe("the sidebar header shows the workspace's own branding (ADM-2)", () => {
  it("shows the workspace name and logo from settings, not a fixed product name", async () => {
    stubApi({
      workspaceName: "Acme Data",
      logo: { type: "image/png", version: "v1" },
      logoDark: { type: "image/png", version: "v2" },
      icon: null,
    })
    const { container } = renderSidebar()

    expect(await screen.findByText("Acme Data")).toBeTruthy()
    // The product's own name is nowhere in the header: a workspace that set
    // a name must not still be called RantAI Lakehouse.
    expect(screen.queryByText("RantAI Lakehouse")).toBeNull()

    const shown = pictures(container)
    expect(shown.map((p) => p.getAttribute("src"))).toEqual([
      "/api/branding/images/logo?v=v1",
      "/api/branding/images/logo-dark?v=v2",
    ])
    expect(shown.map((p) => p.getAttribute("alt"))).toEqual(["Acme Data", "Acme Data"])
  })

  it("shows no fixed subtitle under the name, whether or not the workspace set one", async () => {
    stubApi({ workspaceName: "Acme Data", logo: null, logoDark: null, icon: null })
    const named = renderSidebar()
    expect(await screen.findByText("Acme Data")).toBeTruthy()
    // `ADM-2`: the line "Enterprise Lakehouse Console" described the
    // product, not the workspace, so it goes from the header either way.
    expect(named.container.textContent).not.toContain("Enterprise Lakehouse Console")
    cleanup()

    stubApi({ workspaceName: null, logo: null, logoDark: null, icon: null })
    const unnamed = renderSidebar()
    expect(await screen.findByText(PRODUCT_NAME)).toBeTruthy()
    expect(unnamed.container.textContent).not.toContain("Enterprise Lakehouse Console")
  })

  it("shows the workspace's light logo in dark mode too when no dark logo is set (ADM-2-AC3)", async () => {
    stubApi({
      workspaceName: "Acme Data",
      logo: { type: "image/png", version: "v1" },
      logoDark: null,
      icon: null,
    })
    const { container } = renderSidebar()

    expect(await screen.findByText("Acme Data")).toBeTruthy()
    const shown = pictures(container)
    // The second picture is the dark one: with no dark logo set, it shows
    // the workspace's light logo rather than the product's own dark file.
    expect(shown[1].getAttribute("src")).toBe("/api/branding/images/logo?v=v1")
  })

  it("falls back to the product's own name and logos when the workspace set none", async () => {
    const { requested } = stubApi({
      workspaceName: null,
      logo: null,
      logoDark: null,
      icon: null,
    })
    const { container } = renderSidebar()

    await waitFor(() =>
      expect(requested.some((url) => url.includes("/api/branding"))).toBe(true),
    )
    // The name shows once the answer is in (`adm-121`): until then the slot
    // is empty, so the product's name is not drawn twice by a slow answer.
    expect(await screen.findByText(PRODUCT_NAME)).toBeTruthy()
    expect(pictures(container).map((p) => p.getAttribute("src"))).toEqual([
      "/logo-light.svg",
      "/logo-dark.svg",
    ])
  })

  it("falls back to the product's own logo when the workspace's logo fails to load", async () => {
    stubApi({
      workspaceName: "Acme Data",
      logo: { type: "image/png", version: "v1" },
      logoDark: null,
      icon: null,
    })
    const { container } = renderSidebar()

    expect(await screen.findByText("Acme Data")).toBeTruthy()
    const shown = pictures(container)
    fireEvent.error(shown[0])
    // A broken picture must never be left on the page: the product's own
    // file stands in for it.
    expect(shown[0].getAttribute("src")).toBe("/logo-light.svg")
  })

  it("shows no logo at all while the branding answer is on its way", async () => {
    const { release } = stubApi(
      {
        workspaceName: "Acme Data",
        logo: { type: "image/png", version: "v1" },
        logoDark: { type: "image/png", version: "v2" },
        icon: null,
      },
      { hold: true },
    )
    const { container } = renderSidebar()
    // The sidebar is drawn (the session answered); the branding has not.
    await screen.findByText("Dashboards")

    // No picture at all while the answer is on its way: the product's own
    // logo must not stand on the page for a moment before the workspace's
    // own logo lands (`adm-120`).
    expect(container.querySelectorAll("img")).toHaveLength(0)
    expect(container.innerHTML).not.toContain(PRODUCT_LOGO_LIGHT)
    expect(container.innerHTML).not.toContain(PRODUCT_LOGO_DARK)

    release()
    await waitFor(() => expect(pictures(container)).toHaveLength(2))
    expect(pictures(container).map((p) => p.getAttribute("src"))).toEqual([
      "/api/branding/images/logo?v=v1",
      "/api/branding/images/logo-dark?v=v2",
    ])
  })
})

describe("BI-44: the sidebar asks for the install mode only where it may", () => {
  it("fires no deployment request for a principal without settings:write", async () => {
    const { requested } = stubApi({ workspaceName: null, logo: null, logoDark: null, icon: null })
    renderSidebar()
    expect(await screen.findByText("Dashboards")).toBeTruthy()
    expect(requested.some((url) => url.includes("/api/settings/deployment"))).toBe(false)
  })

  it("hides warehouse-only pages for an admin whose deployment is bi-only", async () => {
    const { requested } = stubApi(
      { workspaceName: null, logo: null, logoDark: null, icon: null },
      {
        permissions: ["*:*"],
        deployment: { features: [], installMode: "bi-only", queryEngine: "clickhouse" },
      },
    )
    renderSidebar()
    await waitFor(() =>
      expect(requested.some((url) => url.includes("/api/settings/deployment"))).toBe(true),
    )
    await waitFor(() => expect(screen.queryByRole("button", { name: "Build" })).toBeNull())
    expect(screen.queryByRole("link", { name: /^pipelines/i })).toBeNull()
    // The Data section is still there; its items render when the section
    // is opened, which is a separate user action from hiding Build.
    expect(screen.getByRole("button", { name: "Data" })).toBeTruthy()
  })

  it("keeps the full menu for an admin whose deployment reports full mode", async () => {
    stubApi(
      { workspaceName: null, logo: null, logoDark: null, icon: null },
      { permissions: ["*:*"], deployment: { features: [], installMode: "full" } },
    )
    renderSidebar()
    expect(await screen.findByRole("link", { name: /^pipelines/i })).toBeTruthy()
  })
})
