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
import { afterAll, afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import * as realTileBody from "./tile-body"
import { AuthProvider } from "@/features/auth/auth-provider"
import { chooseOption } from "@/components/patterns/select-field.testing"

// BI-18 part D (T5): the reader, the editor, the picker and the blocks. The
// chart body is the dashboard's own `TileBody`, replaced here by a marker; the
// real module is put back for the files after this one.
const tileBody = { ...realTileBody }
mock.module("./tile-body", () => ({
  ...tileBody,
  TileBody: ({ spec }: { spec: { id: string } }) => <div data-testid={`tile-${spec.id}`} />,
}))
afterAll(() => {
  mock.module("./tile-body", () => tileBody)
})

const { ReportBlocks } = await import("./report-blocks")
const { ReportReaderPage } = await import("./report-reader-page")
const { ReportEditorPage } = await import("./report-editor-page")
const { ReportChartPicker } = await import("./report-chart-picker")

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

type Call = { method: string; url: string; body?: unknown }

/** A console with a person holding `permissions`, and `routes` answering by URL prefix. */
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
    return Promise.resolve(json({}))
  }) as unknown as typeof fetch)
  return calls
}

afterEach(() => {
  cleanup()
  mock.restore()
  pushed.length = 0
})

const view = (level: string, extra: Record<string, unknown> = {}) => ({
  report: {
    id: "r1", title: "Q3 visitors", folderId: "", createdBy: "Rina", createdAt: "2026-11-13T00:00:00Z",
    updatedBy: "Rina", updatedAt: "2026-11-13T00:00:00Z",
    blocks: [
      { type: "heading", text: "Summary", level: 1 },
      { type: "paragraph", text: "Visits rose." },
      { type: "chart", chart: "c_one", board: "b_1" },
      { type: "chart", chart: "c_hidden", board: "b_2" },
    ],
  },
  charts: {
    c_one: { spec: { id: "c_one", title: "Visits by month", kind: "bar", source: "ui" }, cell: { columns: [], rows: [] }, board: "b_1", boardName: "Sales" },
    c_hidden: { unavailable: "This chart is not available." },
  },
  reporting: null,
  access: { level },
  maxCharts: 30,
  pdfSupported: true,
  canDownload: true,
  ...extra,
})

describe("ReportBlocks", () => {
  it("renders every string as text: markup in a paragraph is shown, not run", () => {
    const { container } = render(
      <ReportBlocks
        blocks={[
          { type: "paragraph", text: "<script>alert(1)</script><b>bold</b>" },
          { type: "link", text: "Run me", url: "javascript:alert(1)" },
          { type: "link", text: "Method", url: "https://example.com/m" },
        ]}
        charts={{}}
        dark={false}
      />,
    )
    expect(container.querySelector("script")).toBeNull()
    expect(container.querySelector("b")).toBeNull()
    expect(screen.getByText("<script>alert(1)</script><b>bold</b>")).toBeTruthy()
    // A link that could run code is words, never an anchor.
    expect(screen.getByText("Run me").closest("a")).toBeNull()
    expect(screen.getByRole("link", { name: "Method" }).getAttribute("href")).toBe("https://example.com/m")
    expect(screen.getByRole("link", { name: "Method" }).getAttribute("rel")).toBe("noopener noreferrer")
  })
})

describe("ReportReaderPage", () => {
  it("draws the blocks in order, each chart live and one the reader cannot see as the sentence", async () => {
    serve(["dashboard:read"], { "GET /api/dashboard/reports/r1": () => json(view("view")) })
    render(<AuthProvider><ReportReaderPage id="r1" /></AuthProvider>)
    expect(await screen.findByRole("heading", { name: "Q3 visitors" })).toBeTruthy()
    expect(screen.getByRole("heading", { name: "Summary" })).toBeTruthy()
    expect(screen.getByTestId("tile-c_one")).toBeTruthy()
    expect(screen.getByText("This chart is not available.")).toBeTruthy()
    expect(screen.getByRole("link", { name: /Sales/ }).getAttribute("href")).toBe("/dashboards/b_1")
  })

  it("offers a reader with dashboard:read only the PDF, never Edit or Delete", async () => {
    serve(["dashboard:read"], { "GET /api/dashboard/reports/r1": () => json(view("view")) })
    render(<AuthProvider><ReportReaderPage id="r1" /></AuthProvider>)
    await screen.findByRole("heading", { name: "Q3 visitors" })
    expect(screen.getByRole("button", { name: /Download PDF/ })).toBeTruthy()
    expect(screen.queryByText("Edit")).toBeNull()
    expect(screen.queryByRole("button", { name: /Delete/ })).toBeNull()
  })

  it("offers Edit and Delete to a writer who manages the folder, and Delete asks first", async () => {
    const calls = serve(["dashboard:read", "dashboard:write"], {
      "GET /api/dashboard/reports/r1": () => json(view("manage")),
      "DELETE /api/dashboard/reports/r1": () => json({ ok: true, id: "r1", retentionDays: 30 }),
    })
    render(<AuthProvider><ReportReaderPage id="r1" /></AuthProvider>)
    await screen.findByRole("heading", { name: "Q3 visitors" })
    expect(screen.getByText("Edit").closest("a")?.getAttribute("href")).toBe("/dashboards/reports/r1/edit")
    fireEvent.click(screen.getByRole("button", { name: /Delete/ }))
    const dialog = await screen.findByRole("alertdialog").catch(() => screen.findByRole("dialog"))
    expect(within(dialog).getByText(/goes to the trash/)).toBeTruthy()
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }))
    await waitFor(() => expect(calls.some((c) => c.method === "DELETE")).toBe(true))
    await waitFor(() => expect(pushed).toContain("/dashboards/browse?tab=reports"))
  })

  it("hides the PDF when the deployment cannot make one", async () => {
    serve(["dashboard:read"], { "GET /api/dashboard/reports/r1": () => json(view("view", { pdfSupported: false })) })
    render(<AuthProvider><ReportReaderPage id="r1" /></AuthProvider>)
    await screen.findByRole("heading", { name: "Q3 visitors" })
    expect(screen.queryByRole("button", { name: /Download PDF/ })).toBeNull()
  })

  it("says a report it cannot read is not available, with the way back", async () => {
    serve(["dashboard:read"], { "GET /api/dashboard/reports/r9": () => json({ error: "report not found." }, 404) })
    render(<AuthProvider><ReportReaderPage id="r9" /></AuthProvider>)
    expect(await screen.findByText("This report is not available")).toBeTruthy()
    expect(screen.getByText("All reports").closest("a")?.getAttribute("href")).toBe("/dashboards/browse?tab=reports")
  })
})

describe("ReportEditorPage", () => {
  it("tells a person without dashboard:write that they cannot write reports", async () => {
    serve(["dashboard:read"], {})
    render(<AuthProvider><ReportEditorPage /></AuthProvider>)
    expect(await screen.findByText("You cannot write reports")).toBeTruthy()
  })

  it("adds blocks, saves them in order and opens the report", async () => {
    const calls = serve(["dashboard:read", "dashboard:write"], {
      "POST /api/dashboard/reports": (call) =>
        json({ ok: true, report: { id: "r2", ...(call.body as object), createdBy: "", createdAt: "", updatedBy: "", updatedAt: "" } }),
    })
    render(<AuthProvider><ReportEditorPage /></AuthProvider>)
    fireEvent.change(await screen.findByLabelText("Title"), { target: { value: "  New one " } })
    fireEvent.click(screen.getByRole("button", { name: "Add block" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: /Heading/ }))
    fireEvent.change(screen.getByLabelText("Heading text"), { target: { value: "Intro" } })
    fireEvent.click(screen.getByRole("button", { name: "Add block" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: /Paragraph/ }))
    fireEvent.change(screen.getByLabelText("Paragraph text"), { target: { value: "<b>as typed</b>" } })
    // Move the paragraph above the heading.
    fireEvent.click(screen.getAllByRole("button", { name: "Move up" })[1])
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true))
    const posted = calls.find((c) => c.method === "POST")!.body as { title: string; folderId: string; blocks: unknown[] }
    expect(posted.title).toBe("New one")
    expect(posted.folderId).toBe("")
    expect(posted.blocks).toEqual([
      { type: "paragraph", text: "<b>as typed</b>" },
      { type: "heading", text: "Intro", level: 2 },
    ])
    await waitFor(() => expect(pushed).toContain("/dashboards/reports/r2"))
  })

  it("shows the server's refusal as it comes and keeps the page", async () => {
    serve(["dashboard:read", "dashboard:write"], {
      "GET /api/dashboard/reports/r1": () => json(view("edit")),
      "PUT /api/dashboard/reports/r1": () => json({ error: "A report holds at most 30 charts." }, 400),
    })
    render(<AuthProvider><ReportEditorPage id="r1" /></AuthProvider>)
    expect(await screen.findByDisplayValue("Q3 visitors")).toBeTruthy()
    expect(screen.getByTestId("chart-count").textContent).toBe("2 of 30 charts")
    // A chart already in the report is drawn live in the editor too.
    expect(screen.getByTestId("tile-c_one")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    expect((await screen.findByRole("alert")).textContent).toBe("A report holds at most 30 charts.")
    expect(pushed).toHaveLength(0)
  })

  it("keeps Save off while a link's address could run code", async () => {
    serve(["dashboard:read", "dashboard:write"], {})
    render(<AuthProvider><ReportEditorPage /></AuthProvider>)
    fireEvent.change(await screen.findByLabelText("Title"), { target: { value: "Links" } })
    fireEvent.click(screen.getByRole("button", { name: "Add block" }))
    fireEvent.click(await screen.findByRole("menuitem", { name: /Link/ }))
    fireEvent.change(screen.getByLabelText("Link address"), { target: { value: "javascript:alert(1)" } })
    expect(screen.getByText(/Start the address with https:\/\//)).toBeTruthy()
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true)
  })
})

describe("ReportChartPicker", () => {
  it("lists a dashboard's data charts and hands back the one picked", async () => {
    serve(["dashboard:read", "dashboard:write"], {
      "GET /api/dashboard/boards": () => json({ boards: [{ id: "default", name: "Main", builtin: true }, { id: "b_1", name: "Sales" }] }),
      "GET /api/dashboard/specs": () =>
        json({
          charts: [
            { id: "c_one", title: "Visits", board: "b_1", kind: "bar" },
            { id: "c_note", title: "A note", board: "b_1", kind: "text" },
            { id: "c_other", title: "Elsewhere", board: "b_2", kind: "bar" },
          ],
        }),
    })
    let picked: unknown = null
    render(
      <AuthProvider>
        <ReportChartPicker open onOpenChange={() => {}} onPick={(p) => { picked = p }} />
      </AuthProvider>,
    )
    const trigger = await screen.findByRole("combobox", { name: "Dashboard" })
    fireEvent.click(trigger)
    expect(screen.queryByRole("option", { name: "Main" })).toBeNull()
    fireEvent.click(trigger)
    chooseOption(trigger, "Sales")
    expect(await screen.findByText("Visits")).toBeTruthy()
    expect(screen.queryByText("A note")).toBeNull()
    expect(screen.queryByText("Elsewhere")).toBeNull()
    fireEvent.click(screen.getByText("Visits"))
    fireEvent.click(screen.getByRole("button", { name: "Add chart" }))
    expect(picked).toEqual({ chart: "c_one", board: "b_1", title: "Visits", boardName: "Sales" })
  })
})
