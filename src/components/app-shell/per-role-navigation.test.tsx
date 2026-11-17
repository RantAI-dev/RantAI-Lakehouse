let currentPathname = "/dashboards"
const mockRouter = {
  push: mock(() => {}),
  replace: mock(() => {}),
  refresh: mock(() => {}),
  back: mock(() => {}),
  forward: mock(() => {}),
  prefetch: mock(() => {}),
}

mock.module("next/navigation", () => ({
  usePathname: () => currentPathname,
  useSearchParams: () => new URLSearchParams(""),
  useRouter: () => mockRouter,
}))

mock.module("next/image", () => ({
  default: () => null,
}))

import * as React from "react"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { AppFrame } from "@/components/app-shell/app-frame"
import { CommandPalette } from "@/components/command-palette"
import { notificationsService } from "@/services"

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })

const originalFetch = global.fetch

function stubAuth(permissions: string[], roleName = "Viewer") {
  const requestedUrls: string[] = []
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = String(input)
    requestedUrls.push(url)
    if (url.includes("/api/auth/me")) {
      return json({
        id: "u1",
        name: "Test User",
        email: "test@rantai.dev",
        roles: [roleName],
        permissions,
        tenants: [{ id: "t1", name: "Default Tenant", role: roleName }],
      })
    }
    return json({})
  }) as unknown as typeof fetch
  return requestedUrls
}

beforeEach(() => {
  mockRouter.push.mockClear()
  mockRouter.replace.mockClear()
  spyOn(notificationsService, "list").mockResolvedValue({
    supported: true,
    openAlerts: [],
    pendingApprovals: [],
    unreadMentions: [],
    unreadMentionsUnavailable: null,
  } as never)
  spyOn(notificationsService, "markMentionsRead").mockResolvedValue(undefined as never)
})

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

import { TooltipProvider } from "@/components/ui/tooltip"

function renderWithProviders(ui: React.ReactNode) {
  return render(
    <TooltipProvider>
      <AuthProvider>
        {ui}
      </AuthProvider>
    </TooltipProvider>
  )
}

describe("ADM-13 D8: per-role navigation and route guard", () => {
  it("1. Dashboard Viewer role sees only Dashboards in sidebar; no dock", async () => {
    currentPathname = "/dashboards"
    stubAuth(["dashboard:read"], "Dashboard Viewer")

    renderWithProviders(
      <AppFrame>
        <div>Dashboard content</div>
      </AppFrame>
    )

    // Wait for auth to settle and chrome to render
    expect(await screen.findByText("Dashboard content")).toBeTruthy()

    // Sidebar: Dashboards is visible
    expect(screen.getByRole("link", { name: /^dashboards/i })).toBeTruthy()

    // Other items are hidden
    expect(screen.queryByRole("link", { name: /^query studio/i })).toBeNull()
    expect(screen.queryByRole("link", { name: /^home/i })).toBeNull()
    expect(screen.queryByRole("link", { name: /^history/i })).toBeNull()
    expect(screen.queryByRole("link", { name: /^catalog/i })).toBeNull()
    expect(screen.queryByRole("link", { name: /^sources/i })).toBeNull()
    expect(screen.queryByRole("link", { name: /^alerts/i })).toBeNull()

    // No dock is rendered
    expect(screen.queryByRole("textbox", { name: /message ai copilot/i })).toBeNull()
    expect(screen.queryByRole("button", { name: /expand copilot/i })).toBeNull()
  })

  it("2. Opening /query-studio directly as Dashboard Viewer renders PermissionState and fires no query-studio API call", async () => {
    currentPathname = "/query-studio"
    const requests = stubAuth(["dashboard:read"], "Dashboard Viewer")

    let childMounted = false
    function QueryStudioChild() {
      React.useEffect(() => {
        childMounted = true
        fetch("/api/query/saved")
      }, [])
      return <div>Query studio content</div>
    }

    renderWithProviders(
      <AppFrame>
        <QueryStudioChild />
      </AppFrame>
    )

    // Route guard blocks child and shows PermissionState
    expect(await screen.findByText("You don't have access")).toBeTruthy()
    expect(
      screen.getByText(/Your role does not permit viewing this resource/)
    ).toBeTruthy()

    // Child component never mounted and never fired API calls
    expect(childMounted).toBe(false)
    expect(screen.queryByText("Query studio content")).toBeNull()
    expect(requests.some((url) => url.includes("/api/query"))).toBe(false)
  })

  it("3. Opening / as Dashboard Viewer redirects to /dashboards", async () => {
    currentPathname = "/"
    stubAuth(["dashboard:read"], "Dashboard Viewer")

    renderWithProviders(
      <AppFrame>
        <div>Home content</div>
      </AppFrame>
    )

    await waitFor(() => {
      expect(mockRouter.replace).toHaveBeenCalledWith("/dashboards")
    })
    expect(screen.queryByText("Home content")).toBeNull()
  })

  it("4. Assistant only user sees Home, History, Dashboards and the dock", async () => {
    currentPathname = "/dashboards"
    stubAuth(["assistant:ask", "dashboard:read"], "Assistant only")

    renderWithProviders(
      <AppFrame>
        <div>Dashboard content</div>
      </AppFrame>
    )

    expect(await screen.findByText("Dashboard content")).toBeTruthy()

    // Allowed links in sidebar
    expect(screen.getByRole("link", { name: /^home/i })).toBeTruthy()
    expect(screen.getByRole("link", { name: /^history/i })).toBeTruthy()
    expect(screen.getByRole("link", { name: /^dashboards/i })).toBeTruthy()

    // Disallowed links
    expect(screen.queryByRole("link", { name: /^query studio/i })).toBeNull()
    expect(screen.queryByRole("link", { name: /^catalog/i })).toBeNull()
    expect(screen.queryByRole("link", { name: /^sources/i })).toBeNull()
    expect(screen.queryByRole("link", { name: /^alerts/i })).toBeNull()

    // Copilot dock is rendered
    expect(screen.getByRole("textbox", { name: /message ai copilot/i })).toBeTruthy()
  })

  it("5. Platform Admin sees everything", async () => {
    currentPathname = "/dashboards"
    stubAuth(["*:*"], "Platform Admin")

    renderWithProviders(
      <AppFrame>
        <div>Dashboard content</div>
      </AppFrame>
    )

    expect(await screen.findByText("Dashboard content")).toBeTruthy()

    // All major sections and links are present
    expect(screen.getByRole("link", { name: /^home/i })).toBeTruthy()
    expect(screen.getByRole("link", { name: /^dashboards/i })).toBeTruthy()
    expect(screen.getByRole("link", { name: /^history/i })).toBeTruthy()
    expect(screen.getByRole("link", { name: /^catalog/i })).toBeTruthy()
    expect(screen.getByRole("link", { name: /^query studio/i })).toBeTruthy()
    expect(screen.getByRole("link", { name: /^sources/i })).toBeTruthy()
    expect(screen.getByRole("link", { name: /^pipelines/i })).toBeTruthy()
    expect(screen.getByText("Governance")).toBeTruthy()
    expect(screen.getByText("Monitoring")).toBeTruthy()
    expect(screen.getByText("Administration")).toBeTruthy()

    // Dock is rendered
    expect(screen.getByRole("textbox", { name: /message ai copilot/i })).toBeTruthy()
  })

  it("6. Command palette hides 'Add a chart' without dashboard:write", async () => {
    // Case A: Viewer without dashboard:write
    stubAuth(["dashboard:read"], "Dashboard Viewer")

    const { unmount } = renderWithProviders(
      <CommandPalette />
    )

    // Open palette
    fireEvent.keyDown(document, { key: "k", ctrlKey: true })
    expect(await screen.findByText("Browse all dashboards")).toBeTruthy()
    expect(screen.queryByText("Add a chart to a dashboard")).toBeNull()

    unmount()

    // Case B: Editor with dashboard:write
    stubAuth(["dashboard:read", "dashboard:write"], "Editor")

    renderWithProviders(
      <CommandPalette />
    )

    fireEvent.keyDown(document, { key: "k", ctrlKey: true })
    expect(await screen.findByText("Browse all dashboards")).toBeTruthy()
    expect(screen.getByText("Add a chart to a dashboard")).toBeTruthy()
  })
})
