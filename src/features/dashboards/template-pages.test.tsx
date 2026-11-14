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
import { DashboardActionsMenu } from "./dashboard-actions"
import { DashboardListPage } from "./dashboard-list-page"
import { SaveTemplateDialog } from "./template-dialogs"

// BI-18 part D (T6): the templates, their dialogs and "Save as template".

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

type Call = { method: string; url: string; body?: unknown }

function serve(permissions: string[], routes: Record<string, (call: Call) => Response>) {
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
      return Promise.resolve(json({ id: "u1", name: "Ana", email: null, roles: ["R"], permissions, tenants: [] }))
    }
    const key = Object.keys(routes)
      .filter((k) => {
        const [method, prefix] = k.split(" ")
        return method === call.method && url.startsWith(prefix)
      })
      .sort((a, b) => b.length - a.length)[0]
    if (key) return Promise.resolve(routes[key](call))
    if (url.includes("/api/dashboard/folders")) return Promise.resolve(json({ folders: [] }))
    if (url.includes("/api/dashboard/boards")) return Promise.resolve(json({ boards: [] }))
    if (url.includes("/api/dashboard/shortcuts")) return Promise.resolve(json({ favourites: [], recent: [] }))
    if (url.includes("/api/dashboard/reports")) return Promise.resolve(json({ reports: [], truncated: false, maxCharts: 30 }))
    if (url.includes("/api/dashboard/templates")) return Promise.resolve(json({ templates: [], truncated: false }))
    return Promise.resolve(json({}))
  }) as unknown as typeof fetch)
  return calls
}

afterEach(() => {
  cleanup()
  mock.restore()
  pushed.length = 0
  // BI-59 T4: the page remembers Table or Gallery; each test starts on the table.
  localStorage.clear()
})

const template = {
  id: "t1", name: "Sales overview", sourceBoard: "b_src", sourceBoardName: "Quarterly sales",
  chartCount: 3, createdBy: "Rina", createdAt: "2026-11-13T00:00:00Z",
}

// BI-59 T2: the templates are rows of the one list, under the Templates filter.
async function openTemplates() {
  render(<AuthProvider><TableProviders><DashboardListPage /></TableProviders></AuthProvider>)
  fireEvent.click(await screen.findByRole("button", { name: "Templates" }))
}

describe("the templates on the Dashboards page", () => {
  it("lists each template with its source, who saved it and its charts", async () => {
    serve(["dashboard:write"], { "GET /api/dashboard/templates": () => json({ templates: [template], truncated: false }) })
    await openTemplates()
    const row = (await screen.findByRole("button", { name: "Sales overview" })).closest("tr")
    if (!row) throw new Error("no row")
    expect(within(row).getByText("Template")).toBeTruthy()
    expect(within(row).getByText("Rina")).toBeTruthy()
    // BI-59 T4: the gallery card says where it came from, who saved it and its charts.
    fireEvent.click(screen.getByRole("button", { name: "Gallery view" }))
    const card = screen.getByRole("button", { name: "Sales overview" }).closest("li")
    if (!card) throw new Error("no card")
    expect(within(card).getByText("Saved from Quarterly sales")).toBeTruthy()
    expect(within(card).getByText(/Rina · 3 charts/)).toBeTruthy()
    // The New menu shows each template with its charts and its source.
    fireEvent.click(screen.getByRole("button", { name: "New" }))
    expect(await screen.findByText("3 charts · from Quarterly sales")).toBeTruthy()
  })

  it("says how to make one when there are none", async () => {
    serve(["dashboard:write"], { "GET /api/dashboard/templates": () => json({ templates: [], truncated: false }) })
    await openTemplates()
    expect(await screen.findByText("No templates yet")).toBeTruthy()
    expect(screen.getByText(/Save as template/)).toBeTruthy()
  })

  it("Use template asks for a name, makes the dashboard and opens it", async () => {
    const calls = serve(["dashboard:write"], {
      "GET /api/dashboard/templates": () => json({ templates: [template], truncated: false }),
      "POST /api/dashboard/templates/t1/use": () => json({ ok: true, board: { id: "b_new", name: "Q4" }, leftOut: [] }),
    })
    await openTemplates()
    fireEvent.click(await screen.findByRole("button", { name: "Sales overview" }))
    const name = await screen.findByLabelText("Name")
    expect((name as HTMLInputElement).value).toBe("Sales overview")
    fireEvent.change(name, { target: { value: "Q4" } })
    fireEvent.click(screen.getByRole("button", { name: "Create dashboard" }))
    await waitFor(() => expect(pushed).toContain("/dashboards/b_new"))
    // The page also opens the writer's personal space with a POST, so the use is found by its address.
    expect(calls.find((c) => c.method === "POST" && c.url.includes("/templates/t1/use"))?.body).toEqual({ name: "Q4", folderId: "" })
  })

  it("shows the server's refusal in the dialog and opens nothing", async () => {
    serve(["dashboard:write"], {
      "GET /api/dashboard/templates": () => json({ templates: [template], truncated: false }),
      "POST /api/dashboard/templates/t1/use": () => json({ error: "permission_denied: edit on this dashboard" }, 403),
    })
    await openTemplates()
    fireEvent.click(await screen.findByRole("button", { name: "Sales overview" }))
    fireEvent.click(await screen.findByRole("button", { name: "Create dashboard" }))
    expect((await screen.findByRole("alert")).textContent).toContain("edit on this dashboard")
    expect(pushed).toHaveLength(0)
  })

  it("deletes a template only after a confirmation", async () => {
    const calls = serve(["dashboard:write"], {
      "GET /api/dashboard/templates": () => json({ templates: [template], truncated: false }),
      "DELETE /api/dashboard/templates/t1": () => json({ ok: true, id: "t1" }),
    })
    await openTemplates()
    fireEvent.click(await screen.findByRole("button", { name: "Actions for Sales overview" }))
    fireEvent.click(await screen.findByText("Delete template…"))
    expect(calls.some((c) => c.method === "DELETE")).toBe(false)
    const dialog = await screen.findByRole("alertdialog").catch(() => screen.findByRole("dialog"))
    expect(within(dialog).getByText(/deleted for good/)).toBeTruthy()
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }))
    await waitFor(() => expect(calls.some((c) => c.method === "DELETE")).toBe(true))
  })
})

describe("SaveTemplateDialog", () => {
  it("saves the dashboard under the name given", async () => {
    const calls = serve(["dashboard:write"], {
      "POST /api/dashboard/templates": () => json({ ok: true, template }),
    })
    let saved: unknown = null
    render(
      <AuthProvider>
        <SaveTemplateDialog open onOpenChange={() => {}} board="b_src" boardName="Quarterly sales" onSaved={(t) => { saved = t }} />
      </AuthProvider>,
    )
    const name = screen.getByLabelText("Template name")
    expect((name as HTMLInputElement).value).toBe("Quarterly sales")
    fireEvent.change(name, { target: { value: "Sales overview" } })
    fireEvent.click(screen.getByRole("button", { name: "Save template" }))
    await waitFor(() => expect(saved).toEqual(template))
    expect(calls.find((c) => c.method === "POST")?.body).toEqual({ board: "b_src", name: "Sales overview" })
  })
})

describe("Save as template in the dashboard's menu", () => {
  const base: React.ComponentProps<typeof DashboardActionsMenu> = {
    isDefault: false, canWrite: true, loading: false, fullscreen: false, autoSec: "0", savedSec: "300",
    canSaveRefresh: false, savingRefresh: false,
    onRefresh: () => {}, onToggleFullscreen: () => {}, onAutoSec: () => {}, onSaveRefresh: () => {},
    onRename: () => {}, onShare: () => {}, onExportPdf: () => {}, onDuplicate: () => {}, onDelete: () => {},
  }

  it("is offered to a writer on a saved dashboard", async () => {
    let opened = false
    render(<DashboardActionsMenu {...base} onSaveTemplate={() => { opened = true }} />)
    fireEvent.click(screen.getByRole("button", { name: "More actions" }))
    fireEvent.click(await screen.findByText("Save as template…"))
    expect(opened).toBe(true)
  })

  it("is not offered without edit, with downloads off, nor on the built-in dashboard", async () => {
    for (const over of [
      { onSaveTemplate: undefined },
      { isDefault: true, onSaveTemplate: () => {} },
      { canWrite: false, onSaveTemplate: () => {} },
      // BI-18 D review round 1, BLOCKER 1: a template follows the download switch.
      { canDownload: false, onSaveTemplate: () => {} },
    ]) {
      render(<DashboardActionsMenu {...base} {...over} />)
      fireEvent.click(screen.getByRole("button", { name: "More actions" }))
      await screen.findByText("Print…")
      expect(screen.queryByText("Save as template…")).toBeNull()
      cleanup()
    }
  })
})

// BI-59 T2: the segmented type filter replaced the tabs.
describe("the Dashboards page type filter", () => {
  it("offers All, Dashboards, Reports and Templates to a writer", async () => {
    serve(["dashboard:read", "dashboard:write"], {})
    render(<AuthProvider><TableProviders><DashboardListPage /></TableProviders></AuthProvider>)
    expect(await screen.findByRole("button", { name: "Templates" })).toBeTruthy()
    for (const name of ["All", "Dashboards", "Reports"]) expect(screen.getByRole("button", { name })).toBeTruthy()
  })

  it("offers a reader no Templates and no New report", async () => {
    serve(["dashboard:read"], {})
    render(<AuthProvider><TableProviders><DashboardListPage /></TableProviders></AuthProvider>)
    const reports = await screen.findByRole("button", { name: "Reports" })
    expect(screen.queryByRole("button", { name: "Templates" })).toBeNull()
    fireEvent.click(reports)
    expect(await screen.findByText("Nobody has shared a report with you yet.")).toBeTruthy()
    expect(screen.queryByText("New report")).toBeNull()
  })
})
