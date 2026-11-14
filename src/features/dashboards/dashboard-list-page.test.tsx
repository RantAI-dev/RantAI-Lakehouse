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
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByText("A canvas of charts, KPIs and tables")).toBeTruthy()
    expect(within(menu).getByText("A written page with live charts")).toBeTruthy()
    expect(await within(menu).findByText("Sales overview")).toBeTruthy()
    expect(within(menu).getByText("Import a dashboard file…")).toBeTruthy()
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

const REPORT = {
  id: "r1", title: "Wisman 2020-2024", folderId: "f1", createdBy: "Bima", updatedBy: "Bima",
  updatedAt: "2026-11-14T08:00:00Z", blockCount: 4, chartCount: 2,
}
const FOLDERS = [{ id: "f1", name: "Tourism", parentId: "", createdBy: "u1" }]

/** The names in the table, top to bottom. */
function rowNames(): string[] {
  return [...document.querySelectorAll("tbody tr")].map((tr) => tr.querySelector("td a, td button")?.textContent?.trim() ?? "")
}

describe("the one list (BI-59 T2)", () => {
  function serveList() {
    return serve(["dashboard:read", "dashboard:write"], {
      "GET /api/dashboard/reports": () => json({ reports: [REPORT], truncated: false, maxCharts: 30 }),
      "GET /api/dashboard/folders": () => json({ folders: FOLDERS }),
    })
  }

  it("holds dashboards, reports and templates, newest first, and each filter keeps only its kind", async () => {
    serveList()
    renderPage()
    await screen.findByText("Wisman 2020-2024")
    expect(rowNames()).toEqual(["Wisman 2020-2024", "QA BI", "Sales overview", "Main"])
    fireEvent.click(screen.getByRole("button", { name: "Reports" }))
    expect(rowNames()).toEqual(["Wisman 2020-2024"])
    fireEvent.click(screen.getByRole("button", { name: "Templates" }))
    expect(rowNames()).toEqual(["Sales overview"])
    fireEvent.click(screen.getByRole("button", { name: "Dashboards" }))
    expect(rowNames()).toEqual(["QA BI", "Main"])
  })

  it("sorts by a column header, and the sort combines with the folder picker", async () => {
    serveList()
    renderPage()
    await screen.findByText("Wisman 2020-2024")
    fireEvent.click(screen.getByRole("button", { name: "Name" }))
    expect(rowNames()).toEqual(["Main", "QA BI", "Sales overview", "Wisman 2020-2024"])
    fireEvent.click(screen.getByRole("button", { name: "Name" }))
    expect(rowNames()).toEqual(["Wisman 2020-2024", "Sales overview", "QA BI", "Main"])
    fireEvent.click(screen.getByRole("button", { name: "Folder: All folders" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: /Tourism/ }))
    await waitFor(() => expect(rowNames()).toEqual(["Wisman 2020-2024"]))
    fireEvent.click(screen.getByRole("button", { name: "Folder: Tourism" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: /No folder/ }))
    await waitFor(() => expect(rowNames()).toEqual(["QA BI", "Main"]))
  })

  it("gives every row the ⋯ menu in the same place, Main included", async () => {
    serveList()
    renderPage()
    await screen.findByText("Wisman 2020-2024")
    for (const tr of document.querySelectorAll("tbody tr")) {
      const last = tr.querySelector("td:last-child")
      expect(last?.querySelector('button[aria-label^="Actions for"]')).toBeTruthy()
    }
    expect(screen.getByRole("button", { name: "Actions for Main" })).toBeTruthy()
  })
})

describe("the pictures and Recent (BI-59 T3)", () => {
  it("reads the chart definitions once for the whole page and reads no dashboard", async () => {
    const calls = serve(["dashboard:read", "dashboard:write"], {
      "GET /api/dashboard/specs": () => json({ charts: [{ id: "u1", board: "b1", kind: "bar" }, { id: "u2", board: "b1", kind: "line" }] }),
      "GET /api/dashboard/shortcuts": () => json({ favourites: [], recent: [{ kind: "board", id: "b1", name: "QA BI" }] }),
    })
    renderPage()
    await screen.findByRole("heading", { name: "Recent" })
    expect(calls.filter((c) => c.url.startsWith("/api/dashboard/specs"))).toHaveLength(1)
    expect(calls.filter((c) => /^\/api\/dashboard(\?|$)/.test(c.url))).toHaveLength(0)
    expect(document.querySelectorAll("tbody svg").length).toBeGreaterThan(0)
  })

  it("shows the four items opened last as picture cards, leaving out one no longer listed", async () => {
    serve(["dashboard:read"], {
      "GET /api/dashboard/shortcuts": () => json({
        favourites: [],
        recent: [
          { kind: "board", id: "b1", name: "QA BI" },
          { kind: "board", id: "gone", name: "Deleted board" },
          { kind: "chart", id: "u9", name: "Arrivals", board: "b1", boardName: "QA BI" },
          { kind: "board", id: "default", name: "Main" },
        ],
      }),
    })
    renderPage()
    const recent = (await screen.findByRole("heading", { name: "Recent" })).closest("section")
    if (!recent) throw new Error("no section")
    const links = within(recent).getAllByRole("link").map((a) => [a.textContent, a.getAttribute("href")])
    expect(links).toEqual([
      ["QA BI", "/dashboards/b1"],
      ["Arrivals", "/dashboards/b1"],
      ["Main", "/dashboards/default"],
    ])
    expect(within(recent).getByText("Chart on QA BI")).toBeTruthy()
  })
})
