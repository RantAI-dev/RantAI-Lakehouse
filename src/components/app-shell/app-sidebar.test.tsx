// `AppSidebar` reads the route through `next/navigation`, which needs a
// Next.js AppRouter context that a bare happy-dom environment does not
// have, so the module is stubbed at the boundary (the same pattern as
// `per-role-navigation.test.tsx`).
mock.module("next/navigation", () => ({
  usePathname: () => "/catalog",
  useSearchParams: () => new URLSearchParams(""),
  useRouter: () => ({ push: () => {}, replace: () => {}, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }),
}))
mock.module("next/image", () => ({ default: () => null }))

import type { ReactNode } from "react"
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider, useAuth } from "@/features/auth/auth-provider"
import { SidebarProvider } from "@/components/ui/sidebar"
import { TooltipProvider } from "@/components/ui/tooltip"
import { forgetRow, readRow, rememberRow, setRowOwner } from "@/features/dashboards/dashboards-row"
import { readLastBoard, rememberLastBoard } from "@/features/dashboards/last-board"
import {
  PRODUCT_LOGO_DARK,
  PRODUCT_LOGO_LIGHT,
  PRODUCT_NAME,
} from "@/lib/product-branding"
import { AppSidebar } from "./app-sidebar"

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const originalFetch = global.fetch

beforeEach(() => {
  // PR #541 review SHOULD-FIX 1: the remembered row is the signed-in person's, and the session
  // below is "u1"; a test that remembers a row before the session answers names "u1" as `AuthProvider` will.
  setRowOwner("u1")
  // happy-dom starts on `about:blank`: a relative URL has no base to
  // resolve against, so a picture with a relative source counts as broken
  // and the branding falls back to the product's own file. The pictures
  // must resolve the way they do in a browser (the same setup as
  // `public-dashboard.test.tsx`, from main's #602).
  ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(
    "http://localhost/",
  )
})

afterEach(() => {
  cleanup()
  window.localStorage.clear()
  mock.restore()
  global.fetch = originalFetch
  setRowOwner(null)
})

function renderSidebar(open: boolean, extra?: ReactNode, userId = "u1") {
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    if (String(input).includes("/api/auth/me")) {
      return json({ id: userId, name: "Ana", email: "ana@example.invalid", roles: ["R"], permissions: ["catalog:read", "query:read", "dashboard:read"], tenants: [] })
    }
    return json({})
  }) as unknown as typeof fetch)
  return render(
    <AuthProvider>
      <SidebarProvider open={open} onOpenChange={() => {}}>
        <AppSidebar />
        {extra}
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
    // BI-61 T11 review: the dashboard keeps the row's chart icon, not the grid the Browse part uses.
    expect(dash.querySelector("svg")?.getAttribute("class")).not.toContain("lucide-layout-grid")
    expect(dash.querySelector("svg")?.getAttribute("class")).toContain("lucide-chart-column")
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

  // PR #541 review SHOULD-FIX 1 (security): the row holds a title, so it never reaches the next
  // person on the browser.
  it("is emptied when the person signs out", async () => {
    rememberRow("dashboard", "b1", "Q4 restructuring plan")
    function SignOut() {
      const { status, logout } = useAuth()
      return status === "authenticated" ? <button type="button" onClick={() => void logout()}>Sign out now</button> : null
    }
    renderSidebar(true, <SignOut />)
    await screen.findByRole("link", { name: "Dashboard: Q4 restructuring plan" })
    fireEvent.click(screen.getByRole("button", { name: "Sign out now" }))
    await waitFor(() => expect(window.localStorage.getItem("dashboards:menu-row")).toBeNull())
    expect(readRow()).toBeNull()
  })

  it("is not shown to another person who signs in on the same browser, and is removed", async () => {
    setRowOwner("u0")
    rememberRow("dashboard", "b1", "Q4 restructuring plan")
    // A new page load: no session has answered yet.
    setRowOwner(null)
    renderSidebar(true, null, "u2")
    await waitFor(() => expect(window.localStorage.getItem("dashboards:menu-row")).toBeNull())
    expect((await screen.findByRole("link", { name: "Dashboards" })).getAttribute("href")).toBe("/dashboards/browse")
    expect(screen.queryByText("Q4 restructuring plan")).toBeNull()
  })

  it("does not show an entry that names no person (stored before rows had an owner)", async () => {
    window.localStorage.setItem("dashboards:menu-row", JSON.stringify({ kind: "dashboard", id: "b1", name: "Q4 restructuring plan" }))
    renderSidebar(true)
    expect((await screen.findByRole("link", { name: "Dashboards" })).getAttribute("href")).toBe("/dashboards/browse")
    expect(screen.queryByText("Q4 restructuring plan")).toBeNull()
  })

  it("ignores a malformed stored value", async () => {
    window.localStorage.setItem("dashboards:menu-row", "{nope")
    renderSidebar(true)
    expect((await screen.findByRole("link", { name: "Dashboards" })).getAttribute("href")).toBe("/dashboards/browse")
  })
})

// ADM-2 (from main): the header shows the workspace's own name and logo.
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
 * at the header while the answer is still on its way (main's #602).
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

function renderBranded() {
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


describe("the sidebar header shows the workspace's own branding (ADM-2)", () => {
  it("shows the workspace name and logo from settings, not a fixed product name", async () => {
    stubApi({
      workspaceName: "Acme Data",
      logo: { type: "image/png", version: "v1" },
      logoDark: { type: "image/png", version: "v2" },
      icon: null,
    })
    const { container } = renderBranded()

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
    const named = renderBranded()
    expect(await screen.findByText("Acme Data")).toBeTruthy()
    // `ADM-2`: the line "Enterprise Lakehouse Console" described the
    // product, not the workspace, so it goes from the header either way.
    expect(named.container.textContent).not.toContain("Enterprise Lakehouse Console")
    cleanup()

    stubApi({ workspaceName: null, logo: null, logoDark: null, icon: null })
    const unnamed = renderBranded()
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
    const { container } = renderBranded()

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
    const { container } = renderBranded()

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
    const { container } = renderBranded()

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
    const { container } = renderBranded()
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
    renderBranded()
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
    renderBranded()
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
    renderBranded()
    expect(await screen.findByRole("link", { name: /^pipelines/i })).toBeTruthy()
  })
})
