// BI-19 owner QA 2026-10-28, QA-2: the bell's mention list is a notification panel (heading,
// newest first, relative time, "Mark all as read", footer link) whose rows open the Comments.
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/default",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { CopilotProvider } from "@/features/copilot/use-copilot"
import { SidebarProvider } from "@/components/ui/sidebar"
import { notificationsService } from "@/services"
import { AppNavbar, UserMenu } from "./app-navbar"
import { CONSOLE_VERSION } from "@/features/version/console-version"

afterEach(() => {
  cleanup()
  mock.restore()
})

const json = (body: unknown) =>
  new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } })

describe("AppNavbar notification panel (BI-19 QA-2)", () => {
  it("lists mentions newest first with a time, a heading, Mark all as read and the Alerts link", async () => {
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return Promise.resolve(json({ id: "u1", name: "Analyst", email: null, roles: [], permissions: [], tenants: [] }))
      }
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)
    const recent = new Date(Date.now() - 5 * 60_000).toISOString()
    const older = new Date(Date.now() - 3 * 3_600_000).toISOString()
    spyOn(notificationsService, "list").mockResolvedValue({
      supported: true,
      openAlerts: [],
      pendingApprovals: [],
      unreadMentions: [
        { id: "c1", board: "b_old", title: "Ann mentioned you on Old", at: older },
        { id: "c2", board: "b_new", title: "Bob mentioned you on New", at: recent },
      ],
      unreadMentionsUnavailable: null,
    } as never)
    const markRead = spyOn(notificationsService, "markMentionsRead").mockResolvedValue(undefined as never)

    render(
      <AuthProvider>
        <CopilotProvider>
          <SidebarProvider>
            <AppNavbar />
          </SidebarProvider>
        </CopilotProvider>
      </AuthProvider>,
    )

    // Until the list arrives the bell is a plain link to /alerts; it becomes the menu trigger after.
    await waitFor(() => expect(screen.getByRole("button", { name: "Notifications" }).getAttribute("aria-haspopup")).toBe("menu"))
    fireEvent.click(screen.getByRole("button", { name: "Notifications" }))
    const rows = await screen.findAllByRole("menuitem")
    // Newest first: Bob (5m) before Ann (3h), then the footer link.
    expect(rows[0].textContent).toContain("Bob mentioned you on New")
    expect(rows[0].textContent).toContain("5m ago")
    expect(rows[0].getAttribute("href")).toBe("/dashboards/b_new?comments=1")
    expect(rows[1].textContent).toContain("3h ago")
    expect(rows[2].textContent).toBe("Alerts and approvals")
    expect(screen.getByText("Notifications", { selector: "span" })).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "Mark all as read" }))
    await waitFor(() => expect(markRead).toHaveBeenCalledTimes(1))
  })
})

/**
 * The account menu says which build the console is (REL-2 S1-T5): a
 * person reporting a problem, or an operator checking what a host runs,
 * needs the number without opening a console page.
 */
describe("the account menu", () => {
  afterEach(() => cleanup())

  it("shows the console's version", () => {
    render(<UserMenu userName="Ada" userEmail="ada@example.com" onLogout={() => {}} />)
    fireEvent.click(screen.getByRole("button", { name: /Account menu/ }))
    expect(screen.getByText(`Version ${CONSOLE_VERSION}`)).toBeTruthy()
  })
})
