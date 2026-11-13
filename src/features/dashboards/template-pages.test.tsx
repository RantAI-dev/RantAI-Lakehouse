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
import { TemplateList } from "./template-list"

// BI-18 part D (T6): the Templates tab, its dialogs and "Save as template".

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
    if (url.includes("/api/dashboard/boards")) return Promise.resolve(json({ boards: [] }))
    if (url.includes("/api/dashboard/shortcuts")) return Promise.resolve(json({ favourites: [], recent: [] }))
    if (url.includes("/api/dashboard/reports")) return Promise.resolve(json({ reports: [], truncated: false, maxCharts: 30 }))
    return Promise.resolve(json({}))
  }) as unknown as typeof fetch)
  return calls
}

afterEach(() => {
  cleanup()
  mock.restore()
  pushed.length = 0
})

const template = {
  id: "t1", name: "Sales overview", sourceBoard: "b_src", sourceBoardName: "Quarterly sales",
  chartCount: 3, createdBy: "Rina", createdAt: "2026-11-13T00:00:00Z",
}

describe("TemplateList", () => {
  it("lists each template with its source, who saved it and its charts", async () => {
    serve(["dashboard:write"], { "GET /api/dashboard/templates": () => json({ templates: [template], truncated: false }) })
    render(<AuthProvider><TemplateList folders={[]} /></AuthProvider>)
    expect(await screen.findByText("Sales overview")).toBeTruthy()
    expect(screen.getByText("From Quarterly sales")).toBeTruthy()
    expect(screen.getByText("3 charts")).toBeTruthy()
    expect(screen.getByText("Rina")).toBeTruthy()
  })

  it("says how to make one when there are none", async () => {
    serve(["dashboard:write"], { "GET /api/dashboard/templates": () => json({ templates: [], truncated: false }) })
    render(<AuthProvider><TemplateList folders={[]} /></AuthProvider>)
    expect(await screen.findByText("No templates yet")).toBeTruthy()
    expect(screen.getByText(/Save as template/)).toBeTruthy()
  })

  it("Use template asks for a name, makes the dashboard and opens it", async () => {
    const calls = serve(["dashboard:write"], {
      "GET /api/dashboard/templates": () => json({ templates: [template], truncated: false }),
      "POST /api/dashboard/templates/t1/use": () => json({ ok: true, board: { id: "b_new", name: "Q4" }, leftOut: [] }),
    })
    render(<AuthProvider><TemplateList folders={[]} /></AuthProvider>)
    fireEvent.click(await screen.findByRole("button", { name: "Use template" }))
    const name = await screen.findByLabelText("Name")
    expect((name as HTMLInputElement).value).toBe("Sales overview")
    fireEvent.change(name, { target: { value: "Q4" } })
    fireEvent.click(screen.getByRole("button", { name: "Create dashboard" }))
    await waitFor(() => expect(pushed).toContain("/dashboards/b_new"))
    expect(calls.find((c) => c.method === "POST")?.body).toEqual({ name: "Q4", folderId: "" })
  })

  it("shows the server's refusal in the dialog and opens nothing", async () => {
    serve(["dashboard:write"], {
      "GET /api/dashboard/templates": () => json({ templates: [template], truncated: false }),
      "POST /api/dashboard/templates/t1/use": () => json({ error: "permission_denied: edit on this dashboard" }, 403),
    })
    render(<AuthProvider><TemplateList folders={[]} /></AuthProvider>)
    fireEvent.click(await screen.findByRole("button", { name: "Use template" }))
    fireEvent.click(await screen.findByRole("button", { name: "Create dashboard" }))
    expect((await screen.findByRole("alert")).textContent).toContain("edit on this dashboard")
    expect(pushed).toHaveLength(0)
  })

  it("deletes a template only after a confirmation", async () => {
    const calls = serve(["dashboard:write"], {
      "GET /api/dashboard/templates": () => json({ templates: [template], truncated: false }),
      "DELETE /api/dashboard/templates/t1": () => json({ ok: true, id: "t1" }),
    })
    render(<AuthProvider><TemplateList folders={[]} /></AuthProvider>)
    fireEvent.click(await screen.findByRole("button", { name: "Delete Sales overview" }))
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

describe("the Dashboards page tabs", () => {
  it("shows Dashboards, Reports and Templates to a writer", async () => {
    serve(["dashboard:read", "dashboard:write"], {})
    render(<AuthProvider><TableProviders><DashboardListPage /></TableProviders></AuthProvider>)
    expect(await screen.findByRole("tab", { name: /Templates/ })).toBeTruthy()
    expect(screen.getByRole("tab", { name: /Reports/ })).toBeTruthy()
    expect(screen.getByRole("tab", { name: /Dashboards/ })).toBeTruthy()
  })

  it("shows a reader no Templates tab and no New report", async () => {
    serve(["dashboard:read"], {})
    render(<AuthProvider><TableProviders><DashboardListPage /></TableProviders></AuthProvider>)
    const reports = await screen.findByRole("tab", { name: /Reports/ })
    expect(screen.queryByRole("tab", { name: /Templates/ })).toBeNull()
    fireEvent.click(reports)
    expect(await screen.findByText("Nobody has shared a report with you yet.")).toBeTruthy()
    expect(screen.queryByText("New report")).toBeNull()
  })
})
