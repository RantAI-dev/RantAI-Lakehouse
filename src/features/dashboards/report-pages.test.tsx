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
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import * as realTileBody from "./tile-body"
import { AuthProvider } from "@/features/auth/auth-provider"

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
const { ReportChartsProvider } = await import("./report-charts")

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

/** The live editor on the page (Tiptap puts it on its element). */
async function pageEditor(container: HTMLElement): Promise<import("@tiptap/core").Editor> {
  const el = await waitFor(() => {
    const found = container.querySelector(".ProseMirror") as (HTMLElement & { editor?: import("@tiptap/core").Editor }) | null
    if (!found?.editor) throw new Error("no editor yet")
    return found
  })
  return el.editor!
}

const catalogue = {
  "GET /api/dashboard/boards": () => json({ boards: [{ id: "default", name: "Main", builtin: true }, { id: "b_1", name: "Sales" }, { id: "b_2", name: "Ops" }] }),
  "GET /api/dashboard/specs": () =>
    json({
      charts: [
        { id: "c_one", title: "Visits by month", board: "b_1", kind: "bar" },
        { id: "c_note", title: "A note", board: "b_1", kind: "text" },
        { id: "c_ops", title: "Tickets", board: "b_2", kind: "kpi" },
        { id: "c_main", title: "Built in", board: "default", kind: "bar" },
      ],
    }),
  "GET /api/dashboard?": (call: Call) =>
    json({
      charts: call.url.includes("b_2")
        ? [{ id: "c_ops", title: "Tickets", kind: "kpi", source: "ui" }]
        : [{ id: "c_one", title: "Visits by month", kind: "bar", source: "ui" }, { id: "c_note", title: "A note", kind: "text", source: "ui" }],
      results: {},
    }),
}

describe("ReportEditorPage (BI-62)", () => {
  it("tells a person without dashboard:write that they cannot write reports", async () => {
    serve(["dashboard:read"], {})
    render(<AuthProvider><ReportEditorPage /></AuthProvider>)
    expect(await screen.findByText("You cannot write reports")).toBeTruthy()
  })

  it("opens a report as a document: the title is the first line, charts are drawn live", async () => {
    serve(["dashboard:read", "dashboard:write"], { ...catalogue, "GET /api/dashboard/reports/r1": () => json(view("edit")) })
    const { container } = render(<AuthProvider><ReportEditorPage id="r1" /></AuthProvider>)
    await pageEditor(container)
    expect(container.querySelector(".ProseMirror h1.rd-title")?.textContent).toBe("Q3 visitors")
    expect(container.querySelector(".ProseMirror [data-level='1']")?.textContent).toBe("Summary")
    // A chart already in the report is drawn from the report's own read.
    expect(await screen.findByTestId("tile-c_one")).toBeTruthy()
    expect(screen.getByText("This chart is not available.")).toBeTruthy()
    // No labels over the text: nothing reads "Heading" or "Paragraph" on the page.
    expect(screen.queryByText("Paragraph")).toBeNull()
  })

  it("autosaves what was written as blocks, marks as the inline subset, and makes the report once", async () => {
    const calls = serve(["dashboard:read", "dashboard:write"], {
      ...catalogue,
      "POST /api/dashboard/reports": (call) =>
        json({ ok: true, report: { id: "r2", ...(call.body as object), createdBy: "", createdAt: "", updatedBy: "", updatedAt: "" } }),
    })
    const addresses: string[] = []
    spyOn(window.history, "replaceState").mockImplementation((_state, _unused, url) => {
      addresses.push(String(url))
    })
    const { container } = render(<AuthProvider><ReportEditorPage /></AuthProvider>)
    const editor = await pageEditor(container)
    editor.commands.setContent({
      type: "doc",
      content: [
        { type: "title", content: [{ type: "text", text: "New one" }] },
        { type: "heading", attrs: { level: 2 }, content: [{ type: "text", text: "Intro" }] },
        { type: "paragraph", content: [{ type: "text", text: "<b>as typed</b> " }, { type: "text", text: "bold", marks: [{ type: "bold" }] }] },
        { type: "chart", attrs: { chart: "c_one", width: "wide" } },
      ],
    })
    // No Save button: leaving the page saves at once (the timer would, a second later).
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull()
    window.dispatchEvent(new Event("pagehide"))
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true))
    const posted = calls.find((c) => c.method === "POST")!.body as { title: string; folderId: string; blocks: unknown[] }
    expect(posted.title).toBe("New one")
    expect(posted.folderId).toBe("")
    expect(posted.blocks).toEqual([
      { type: "heading", text: "Intro", level: 2 },
      { type: "paragraph", text: "<b>as typed</b> **bold**" },
      { type: "chart", chart: "c_one", width: "wide" },
    ])
    // The address becomes the edit address with no navigation, and the status says Saved.
    await waitFor(() => expect(addresses).toEqual(["/dashboards/reports/r2/edit"]))
    expect(pushed).toHaveLength(0)
    expect(await screen.findByText("Saved")).toBeTruthy()
    // The next change updates the report the first save made.
    editor.commands.insertContentAt(editor.state.doc.content.size, { type: "paragraph", content: [{ type: "text", text: "more" }] })
    window.dispatchEvent(new Event("pagehide"))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT" && c.url.endsWith("/api/dashboard/reports/r2"))).toBe(true))
    expect(calls.filter((c) => c.method === "POST")).toHaveLength(1)
  })

  it("saves nothing when a report is only opened, and an untouched new report is never made", async () => {
    const calls = serve(["dashboard:read", "dashboard:write"], { ...catalogue, "GET /api/dashboard/reports/r1": () => json(view("edit")) })
    const { container, unmount } = render(<AuthProvider><ReportEditorPage id="r1" /></AuthProvider>)
    await pageEditor(container)
    window.dispatchEvent(new Event("pagehide"))
    unmount()
    const fresh = render(<AuthProvider><ReportEditorPage /></AuthProvider>)
    await pageEditor(fresh.container)
    window.dispatchEvent(new Event("pagehide"))
    await new Promise((r) => setTimeout(r, 50))
    expect(calls.filter((c) => c.method === "POST" || c.method === "PUT")).toHaveLength(0)
  })

  it("opens a report made before BI-62 with its link block as a link in a paragraph", async () => {
    const calls = serve(["dashboard:read", "dashboard:write"], {
      ...catalogue,
      "GET /api/dashboard/reports/r1": () =>
        json(view("edit", { report: { ...view("edit").report, blocks: [{ type: "link", text: "Method", url: "https://example.com/m" }] } })),
      "PUT /api/dashboard/reports/r1": (call) => json({ ok: true, report: { ...view("edit").report, ...(call.body as object) } }),
    })
    const { container } = render(<AuthProvider><ReportEditorPage id="r1" /></AuthProvider>)
    const editor = await pageEditor(container)
    expect(container.querySelector(".ProseMirror p a")?.getAttribute("href")).toBe("https://example.com/m")
    // Opening it saves nothing; the first change saves the link as a paragraph holding it.
    editor.commands.insertContentAt(1, " ")
    editor.commands.deleteRange({ from: 1, to: 2 })
    editor.commands.insertContentAt(editor.state.doc.content.size, { type: "paragraph", content: [{ type: "text", text: "Next" }] })
    window.dispatchEvent(new Event("pagehide"))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    expect((calls.find((c) => c.method === "PUT")!.body as { blocks: unknown[] }).blocks).toEqual([
      { type: "paragraph", text: "[Method](https://example.com/m)" },
      { type: "paragraph", text: "Next" },
    ])
  })

  it("shows the server's refusal beside the status, keeps the text, and Retry tries again", async () => {
    let refuse = true
    const calls = serve(["dashboard:read", "dashboard:write"], {
      ...catalogue,
      "GET /api/dashboard/reports/r1": () => json(view("edit")),
      "PUT /api/dashboard/reports/r1": (call) =>
        refuse ? json({ error: "A report holds at most 30 charts." }, 400) : json({ ok: true, report: { ...view("edit").report, ...(call.body as object) } }),
    })
    const { container } = render(<AuthProvider><ReportEditorPage id="r1" /></AuthProvider>)
    const editor = await pageEditor(container)
    editor.commands.insertContentAt(editor.state.doc.content.size, { type: "paragraph", content: [{ type: "text", text: "kept" }] })
    window.dispatchEvent(new Event("pagehide"))
    expect((await screen.findByRole("alert")).textContent).toBe("A report holds at most 30 charts.")
    expect(screen.getByText("Couldn't save")).toBeTruthy()
    expect(container.querySelector(".ProseMirror")?.textContent).toContain("kept")
    refuse = false
    fireEvent.click(screen.getByRole("button", { name: "Retry" }))
    expect(await screen.findByText("Saved")).toBeTruthy()
    expect(calls.filter((c) => c.method === "PUT")).toHaveLength(2)
  })

  it("has no Preview button: the editor already shows the reader's view", async () => {
    serve(["dashboard:read", "dashboard:write"], { ...catalogue, "GET /api/dashboard/reports/r1": () => json(view("edit")) })
    const { container } = render(<AuthProvider><ReportEditorPage id="r1" /></AuthProvider>)
    await pageEditor(container)
    expect(screen.queryByRole("button", { name: "Preview" })).toBeNull()
    expect(screen.getByRole("button", { name: "Charts" })).toBeTruthy()
  })
})

describe("ReportChartPicker (BI-62)", () => {
  // The pictures load when on screen; this observer says every one is. The
  // real global is put back after each test.
  const realObserver = globalThis.IntersectionObserver
  beforeEach(() => {
    globalThis.IntersectionObserver = class {
      constructor(private readonly cb: IntersectionObserverCallback) {}
      observe(el: Element) {
        this.cb([{ isIntersecting: true, target: el } as IntersectionObserverEntry], this as unknown as IntersectionObserver)
      }
      disconnect() {}
      unobserve() {}
      takeRecords() {
        return []
      }
    } as unknown as typeof IntersectionObserver
  })
  afterEach(() => {
    globalThis.IntersectionObserver = realObserver
  })

  it("shows each dashboard's data charts as pictures, searches them, and hands back the one picked", async () => {
    serve(["dashboard:read", "dashboard:write"], catalogue)
    const picked: unknown[] = []
    render(
      <AuthProvider>
        <ReportChartsProvider>
          <ReportChartPicker open onOpenChange={() => {}} used={new Set(["c_ops"])} count={1} onPick={(p) => { picked.push(p) }} />
        </ReportChartsProvider>
      </AuthProvider>,
    )
    expect(await screen.findByRole("region", { name: "Sales" })).toBeTruthy()
    expect(screen.getByRole("region", { name: "Ops" })).toBeTruthy()
    // The built-in dashboard and a tile without data are not offered.
    expect(screen.queryByText("Built in")).toBeNull()
    expect(screen.queryByText("A note")).toBeNull()
    expect(screen.getByText("In report")).toBeTruthy()
    // The pictures draw from each dashboard's own read.
    expect(await screen.findByTestId("tile-c_one")).toBeTruthy()
    fireEvent.change(screen.getByPlaceholderText("Search charts and dashboards"), { target: { value: "tick" } })
    expect(screen.queryByRole("region", { name: "Sales" })).toBeNull()
    fireEvent.click(screen.getByText("Tickets"))
    expect(picked).toEqual([{ id: "c_ops", title: "Tickets", kind: "kpi", board: "b_2" }])
    fireEvent.change(screen.getByPlaceholderText("Search charts and dashboards"), { target: { value: "zzz" } })
    expect(screen.getByText("No charts match “zzz”")).toBeTruthy()
  })

  it("offers nothing more once the report holds 30 charts", async () => {
    serve(["dashboard:read", "dashboard:write"], catalogue)
    render(
      <AuthProvider>
        <ReportChartsProvider>
          <ReportChartPicker open onOpenChange={() => {}} used={new Set()} count={30} onPick={() => {}} />
        </ReportChartsProvider>
      </AuthProvider>,
    )
    expect(await screen.findByText(/This report has 30 charts/)).toBeTruthy()
    expect((screen.getByText("Visits by month").closest("button") as HTMLButtonElement).disabled).toBe(true)
  })
})
