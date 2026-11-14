const pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/browse",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: (href: string) => { pushed.push(href) },
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { DashboardListPage } from "./dashboard-list-page"

// BI-59: the Dashboards page (layout C of the 2026-11-14 mockups).

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

type Call = { method: string; url: string; body?: unknown }

/** Answers the page's reads; `routes` overrides one by "METHOD /prefix". */
function serve(permissions: string[], routes: Record<string, (call: Call) => Response> = {}) {
  const calls: Call[] = []
  spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const call: Call = {
      method: init?.method ?? "GET",
      url,
      body: typeof init?.body === "string" ? JSON.parse(init.body) : undefined,
    }
    calls.push(call)
    if (url.includes("/api/auth/me")) {
      return Promise.resolve(json({ id: "u1", name: "Rina", email: null, roles: ["R"], permissions, tenants: [] }))
    }
    const key = Object.keys(routes)
      .filter((k) => {
        const [method, prefix] = k.split(" ")
        return method === call.method && url.startsWith(prefix)
      })
      .sort((a, b) => b.length - a.length)[0]
    if (key) return Promise.resolve(routes[key](call))
    if (url.includes("/api/dashboard/folders")) return Promise.resolve(json({ folders: [] }))
    if (url.includes("/api/dashboard/boards")) return Promise.resolve(json({ boards: BOARDS }))
    if (url.includes("/api/dashboard/shortcuts")) return Promise.resolve(json({ favourites: [], recent: [] }))
    if (url.includes("/api/dashboard/reports")) return Promise.resolve(json({ reports: [], truncated: false, maxCharts: 30 }))
    if (url.includes("/api/dashboard/templates")) return Promise.resolve(json({ templates: [TEMPLATE], truncated: false }))
    if (url.includes("/api/dashboard/specs")) return Promise.resolve(json({ charts: [] }))
    return Promise.resolve(json({}))
  }) as unknown as typeof fetch)
  return calls
}

afterEach(() => {
  cleanup()
  mock.restore()
  pushed.length = 0
})

const BOARDS = [
  { id: "default", name: "Main", layout: {}, chartCount: 13, builtin: true },
  { id: "b1", name: "QA BI", createdBy: "Rina", updatedAt: "2026-11-13T00:00:00Z", chartCount: 3, builtin: false },
]
const TEMPLATE = {
  id: "t1", name: "Sales overview", sourceBoard: "b_src", sourceBoardName: "Quarterly sales",
  chartCount: 3, createdBy: "Rina", createdAt: "2026-11-13T00:00:00Z",
}

function renderPage() {
  return render(<AuthProvider><TableProviders><DashboardListPage /></TableProviders></AuthProvider>)
}

/** The page header: the title, the page's ⋯, New and the search. */
async function header() {
  await screen.findByRole("heading", { name: "Dashboards" })
  const el = document.querySelector("header")
  if (!el) throw new Error("no header")
  return el
}

describe("the Dashboards page header (BI-59 T1)", () => {
  it("shows a writer one New, one ⋯ and the search, and no other button", async () => {
    serve(["dashboard:read", "dashboard:write"])
    renderPage()
    const head = await header()
    await waitFor(() => expect(within(head).getByRole("button", { name: "New" })).toBeTruthy())
    const buttons = within(head).getAllByRole("button").map((b) => b.getAttribute("aria-label") ?? b.textContent?.trim())
    expect(buttons.sort()).toEqual(["More actions", "New"])
    expect(within(head).getByRole("searchbox")).toBeTruthy()
    // The old toolbar's controls are gone from the page.
    for (const gone of ["Metrics", "Folders", "Trash", "Import…", "New dashboard", "Gallery view"]) {
      expect(screen.queryByRole("button", { name: gone })).toBeNull()
    }
  })

  it("New offers a dashboard, a report, each template and an import", async () => {
    serve(["dashboard:read", "dashboard:write"])
    renderPage()
    const head = await header()
    fireEvent.click(await within(head).findByRole("button", { name: "New" }))
    expect(await screen.findByText("A canvas of charts, KPIs and tables")).toBeTruthy()
    expect(screen.getByText("A written page with live charts")).toBeTruthy()
    expect(await screen.findByText("Sales overview")).toBeTruthy()
    expect(screen.getByText("Import a dashboard file…")).toBeTruthy()
  })

  it("the page's ⋯ holds Metrics, Manage folders and Trash for a writer", async () => {
    serve(["dashboard:read", "dashboard:write"])
    renderPage()
    fireEvent.click(within(await header()).getByRole("button", { name: "More actions" }))
    expect(await screen.findByText("Metrics")).toBeTruthy()
    expect(screen.getByText("Manage folders…")).toBeTruthy()
    expect(screen.getByText("Trash")).toBeTruthy()
  })

  it("shows a reader no New, and Metrics alone in the ⋯", async () => {
    serve(["dashboard:read"])
    renderPage()
    const head = await header()
    await waitFor(() => expect(within(head).getByRole("button", { name: "More actions" })).toBeTruthy())
    expect(within(head).queryByRole("button", { name: "New" })).toBeNull()
    fireEvent.click(within(head).getByRole("button", { name: "More actions" }))
    expect(await screen.findByText("Metrics")).toBeTruthy()
    expect(screen.queryByText("Manage folders…")).toBeNull()
    expect(screen.queryByText("Trash")).toBeNull()
  })

  it('"/" focuses the search and Escape clears it', async () => {
    serve(["dashboard:read"])
    renderPage()
    const search = within(await header()).getByRole("searchbox") as HTMLInputElement
    fireEvent.keyDown(window, { key: "/" })
    expect(document.activeElement).toBe(search)
    fireEvent.change(search, { target: { value: "qa" } })
    fireEvent.keyDown(search, { key: "Escape" })
    expect(search.value).toBe("")
  })
})
