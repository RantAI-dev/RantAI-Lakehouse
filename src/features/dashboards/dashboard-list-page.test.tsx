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
import { chooseOption } from "@/components/patterns/select-field.testing"
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
  // The page remembers Table or Gallery; each test starts on the table.
  localStorage.clear()
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
    for (const gone of ["Metrics", "Folders", "Trash", "Import…", "New dashboard", "Refresh"]) {
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
  return [...document.querySelectorAll("tbody tr")].map((tr) => tr.querySelector("td a, td button:not([role=checkbox])")?.textContent?.trim() ?? "")
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

describe("the Gallery view (BI-59 T4)", () => {
  const specs = { "GET /api/dashboard/specs": () => json({ charts: [{ id: "u1", board: "b1", kind: "bar" }] }) }

  it("opens on the table, and the gallery chosen survives a reload", async () => {
    serve(["dashboard:read", "dashboard:write"], specs)
    const first = renderPage()
    await screen.findByText("QA BI")
    expect(document.querySelector("table")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Gallery view" }))
    expect(document.querySelector("table")).toBeNull()
    expect(screen.getByRole("button", { name: "Gallery view" }).getAttribute("aria-pressed")).toBe("true")
    first.unmount()
    renderPage()
    await screen.findByText("QA BI")
    await waitFor(() => expect(screen.getByRole("button", { name: "Gallery view" }).getAttribute("aria-pressed")).toBe("true"))
    expect(document.querySelector("table")).toBeNull()
  })

  it("shows each dashboard as a picture card, grouped by folder", async () => {
    serve(["dashboard:read", "dashboard:write"], specs)
    renderPage()
    await screen.findByText("QA BI")
    fireEvent.click(screen.getByRole("button", { name: "Gallery view" }))
    const noFolder = screen.getByRole("region", { name: "No folder" })
    const card = within(noFolder).getByText("QA BI").closest("li")
    expect(card?.querySelector('svg[role="presentation"]')).toBeTruthy()
    expect(within(noFolder).getByText("Built-in dashboard")).toBeTruthy()
    expect(screen.getByRole("region", { name: "Templates" })).toBeTruthy()
    expect(screen.queryByText("No description.")).toBeNull()
  })

  it("still renders, on the table, when localStorage throws", async () => {
    serve(["dashboard:read"], specs)
    // Storage blocked (a private window, site data off): reading the
    // property itself throws, as browsers do.
    const own = Object.getOwnPropertyDescriptor(window, "localStorage")
    let reads = 0
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      get() {
        reads += 1
        throw new Error("blocked")
      },
    })
    try {
      renderPage()
      await screen.findByText("QA BI")
      expect(document.querySelector("table")).toBeTruthy()
      fireEvent.click(screen.getByRole("button", { name: "Gallery view" }))
      expect(document.querySelector("table")).toBeNull()
      expect(reads).toBeGreaterThan(0)
    } finally {
      if (own) Object.defineProperty(window, "localStorage", own)
      else delete (window as { localStorage?: Storage }).localStorage
    }
  })
})

describe("the header and toolbar controls (BI-59 AC1)", () => {
  it("are New, the ⋯, the search, the type filter, the folder picker and the view switch, and nothing else", async () => {
    serve(["dashboard:read", "dashboard:write"])
    renderPage()
    await screen.findByText("QA BI")
    const list = document.querySelector("table")
    const recent = screen.queryByRole("heading", { name: "Recent" })?.closest("section")
    const controls = [...document.querySelectorAll("button")]
      .filter((b) => !list?.contains(b) && !recent?.contains(b))
      .map((b) => b.getAttribute("aria-label") ?? b.textContent?.trim())
    expect(controls.sort()).toEqual(
      ["All", "Dashboards", "Folder: All folders", "Gallery view", "More actions", "New", "Reports", "Table view", "Templates"].sort(),
    )
    expect(screen.getAllByRole("searchbox")).toHaveLength(1)
  })
})

describe("bulk actions, facts and empty states (BI-59 T5)", () => {
  const listRoutes = {
    "GET /api/dashboard/reports": () => json({ reports: [REPORT], truncated: false, maxCharts: 30 }),
    "GET /api/dashboard/folders": () => json({ folders: FOLDERS }),
  }

  async function tick(...names: string[]) {
    await screen.findByText("Wisman 2020-2024")
    for (const name of names) fireEvent.click(screen.getByRole("checkbox", { name: `Select ${name}` }))
    return await screen.findByRole("toolbar", { name: "Selected items" })
  }

  it("offers no box to tick on Main or a template, and none at all to a reader", async () => {
    serve(["dashboard:read", "dashboard:write"], listRoutes)
    renderPage()
    await screen.findByText("Wisman 2020-2024")
    expect(screen.queryByRole("checkbox", { name: "Select Main" })).toBeNull()
    expect(screen.queryByRole("checkbox", { name: "Select Sales overview" })).toBeNull()
    expect(screen.getByRole("checkbox", { name: "Select QA BI" })).toBeTruthy()
    cleanup()
    serve(["dashboard:read"], listRoutes)
    renderPage()
    await screen.findByText("Wisman 2020-2024")
    expect(screen.queryAllByRole("checkbox")).toHaveLength(0)
  })

  it("moves the ticked dashboard and report to a folder through the existing services", async () => {
    const calls = serve(["dashboard:read", "dashboard:write"], {
      ...listRoutes,
      "PUT /api/dashboard/boards": () => json({ ok: true }),
      "PUT /api/dashboard/reports": () => json({ report: { ...REPORT, folderId: "" } }),
      "GET /api/dashboard/access-preview": () => json({ gains: [], loses: [], startingPointGains: false, startingPointLoses: false }),
    })
    renderPage()
    const bar = await tick("QA BI", "Wisman 2020-2024")
    expect(within(bar).getByText("2 selected")).toBeTruthy()
    fireEvent.click(within(bar).getByRole("button", { name: /Move to folder/ }))
    const dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByText("2 items")).toBeTruthy()
    // The report sits in Tourism and the dashboard at the top level: Tourism is the target.
    chooseOption(within(dialog).getByRole("combobox", { name: "Folder" }), "Tourism")
    fireEvent.click(within(dialog).getByRole("button", { name: "Move" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT" && c.url === "/api/dashboard/boards")).toBe(true))
    expect(calls.find((c) => c.method === "PUT" && c.url === "/api/dashboard/boards")?.body).toEqual({ id: "b1", folderId: "f1" })
    // The report is already in Tourism, so it is not written again.
    expect(calls.some((c) => c.method === "PUT" && c.url.startsWith("/api/dashboard/reports"))).toBe(false)
  })

  it("favourites the ticked dashboards and skips a report, which cannot be one", async () => {
    const calls = serve(["dashboard:read", "dashboard:write"], {
      ...listRoutes,
      "PUT /api/dashboard/favourites": () => json({ ok: true }),
    })
    renderPage()
    const bar = await tick("QA BI", "Wisman 2020-2024")
    fireEvent.click(within(bar).getByRole("button", { name: /Favourite/ }))
    await waitFor(() => expect(calls.filter((c) => c.method === "PUT" && c.url === "/api/dashboard/favourites")).toHaveLength(1))
    expect(calls.find((c) => c.url === "/api/dashboard/favourites")?.body).toEqual({ kind: "board", id: "b1", on: true })
    await waitFor(() => expect(screen.queryByRole("toolbar", { name: "Selected items" })).toBeNull())
  })

  it("moves the ticked rows to the trash only after a confirmation, and the trash can still restore", async () => {
    const calls = serve(["dashboard:read", "dashboard:write"], {
      ...listRoutes,
      "DELETE /api/dashboard/boards": () => json({ ok: true }),
      "DELETE /api/dashboard/reports": () => json({ ok: true }),
      "GET /api/dashboard/trash": () => json({
        items: [{ kind: "board", id: "b1", name: "QA BI", board: "", boardName: "", boardState: "", deletedBy: "Rina", deletedAt: "2026-11-14T00:00:00Z", expiresAt: "2026-12-14T00:00:00Z", personal: false }],
        retentionDays: 30,
      }),
      "POST /api/dashboard/trash/restore": () => json({ restored: { kind: "board", id: "b1", charts: 0, boardState: "" } }),
    })
    renderPage()
    const bar = await tick("QA BI", "Wisman 2020-2024")
    fireEvent.click(within(bar).getByRole("button", { name: /Move to trash/ }))
    expect(calls.some((c) => c.method === "DELETE")).toBe(false)
    const confirm = await screen.findByRole("alertdialog").catch(() => screen.findByRole("dialog"))
    expect(within(confirm).getByText("Move 2 items to the trash?")).toBeTruthy()
    fireEvent.click(within(confirm).getByRole("button", { name: "Move to trash" }))
    await waitFor(() => expect(calls.filter((c) => c.method === "DELETE")).toHaveLength(2))
    // In the list's order (newest first): the report, then the dashboard.
    expect(calls.filter((c) => c.method === "DELETE").map((c) => c.url)).toEqual([
      "/api/dashboard/reports/r1",
      "/api/dashboard/boards?id=b1",
    ])
    // Restore from the trash still works and reloads the list.
    const boardReads = () => calls.filter((c) => c.method === "GET" && c.url === "/api/dashboard/boards").length
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull())
    fireEvent.click(screen.getByRole("button", { name: "More actions" }))
    fireEvent.click(await screen.findByText("Trash"))
    const before = boardReads()
    fireEvent.click(await screen.findByRole("button", { name: "Restore" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST" && c.url === "/api/dashboard/trash/restore")).toBe(true))
    await waitFor(() => expect(boardReads()).toBeGreaterThan(before))
  })

  it("never says No description.: a description is the card's fact", async () => {
    serve(["dashboard:read"], {
      "GET /api/dashboard/boards": () => json({ boards: [...BOARDS, { id: "b2", name: "Arrivals", description: "Weekly arrivals by port", chartCount: 0 }] }),
    })
    renderPage()
    await screen.findByText("Arrivals")
    fireEvent.click(screen.getByRole("button", { name: "Gallery view" }))
    expect(screen.getByText("Weekly arrivals by port")).toBeTruthy()
    expect(screen.queryByText("No description.")).toBeNull()
  })

  it("says what to do when the search matches nothing, a folder is empty, or there is nothing but Main", async () => {
    serve(["dashboard:read", "dashboard:write"], { "GET /api/dashboard/folders": () => json({ folders: FOLDERS }) })
    renderPage()
    await screen.findByText("QA BI")
    const search = screen.getByRole("searchbox")
    fireEvent.change(search, { target: { value: "zzz" } })
    expect(await screen.findByText("Nothing matches “zzz”")).toBeTruthy()
    // The empty state's own button (the search field has an ✕ of the same name).
    const clear = screen.getAllByRole("button", { name: "Clear search" })
    expect(clear).toHaveLength(2)
    fireEvent.click(clear[1])
    expect((search as HTMLInputElement).value).toBe("")
    fireEvent.click(screen.getByRole("button", { name: "Folder: All folders" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: /Tourism/ }))
    expect(await screen.findByText("Nothing in Tourism yet")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Show all folders" }))
    await screen.findByText("QA BI")
    cleanup()
    serve(["dashboard:read", "dashboard:write"], {
      "GET /api/dashboard/boards": () => json({ boards: [BOARDS[0]] }),
      "GET /api/dashboard/templates": () => json({ templates: [], truncated: false }),
    })
    renderPage()
    expect(await screen.findByText("Nothing made here yet")).toBeTruthy()
    expect(screen.getByText("Main")).toBeTruthy()
  })
})

describe("at phone width (BI-59 T6)", () => {
  it("stacks the rows instead of a table, each with its facts, star and ⋯", async () => {
    const own = Object.getOwnPropertyDescriptor(window, "innerWidth")
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 390 })
    try {
      serve(["dashboard:read", "dashboard:write"], {
        "GET /api/dashboard/reports": () => json({ reports: [REPORT], truncated: false, maxCharts: 30 }),
        "GET /api/dashboard/folders": () => json({ folders: FOLDERS }),
      })
      renderPage()
      const list = await screen.findByRole("list", { name: "Dashboards, reports and templates" })
      expect(document.querySelector("table")).toBeNull()
      const row = within(list).getByText("Wisman 2020-2024").closest("li")
      if (!row) throw new Error("no row")
      // Type, size, folder, owner, then when (a relative time).
      expect(within(row).getByText(/^Report · 2 charts · Tourism · Bima · .+ ago$/)).toBeTruthy()
      expect(within(row).getByRole("button", { name: "Actions for Wisman 2020-2024" })).toBeTruthy()
      const qa = within(list).getByText("QA BI").closest("li")
      if (!qa) throw new Error("no row")
      expect(within(qa).getByRole("button", { name: "Add QA BI to favourites" })).toBeTruthy()
      expect(within(qa).getByRole("checkbox", { name: "Select QA BI" })).toBeTruthy()
    } finally {
      if (own) Object.defineProperty(window, "innerWidth", own)
      else delete (window as { innerWidth?: number }).innerWidth
    }
  })
})
