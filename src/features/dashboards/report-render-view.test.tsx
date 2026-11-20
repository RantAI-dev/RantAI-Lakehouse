import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import * as realTileBody from "./tile-body"

// BI-18 part D (T2): the report's print view. `mock.module` outlives this file
// in `bun test src`, so the real exports are put back in `afterAll`.
const tileBody = { ...realTileBody }
const loaders: (((id: string) => Promise<Response>) | undefined)[] = []
mock.module("./tile-body", () => ({
  ...tileBody,
  TileBody: ({ spec, dark, loadMap }: { spec: { id: string }; dark?: boolean; loadMap?: (id: string) => Promise<Response> }) => {
    loaders.push(loadMap)
    return <div data-testid={`tile-${spec.id}`} data-dark={String(dark === true)} />
  },
}))
afterAll(() => {
  mock.module("./tile-body", () => tileBody)
})

const { ReportRenderView } = await import("./report-render-view")
const { renderFailure } = await import("./render-view")

const originalFetch = global.fetch
const originalRaise = renderFailure.raise
const flag = () => (window as unknown as { __lakehouseRenderReady?: boolean }).__lakehouseRenderReady

beforeEach(() => {
  delete (window as unknown as { __lakehouseRenderReady?: boolean }).__lakehouseRenderReady
})
afterEach(() => {
  cleanup()
  loaders.length = 0
  global.fetch = originalFetch
  renderFailure.raise = originalRaise
})

function serve(body: unknown, status = 200) {
  const calls: { url: string; body: string }[] = []
  global.fetch = mock(async (url: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(url), body: String(init?.body ?? "") })
    return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
  }) as unknown as typeof fetch
  return calls
}

const payload = {
  report: {
    id: "r1", title: "Q3 visitors", folderId: "", createdBy: "Rina", createdAt: "", updatedBy: "Rina", updatedAt: "",
    blocks: [
      { type: "heading", text: "Summary", level: 1 },
      { type: "paragraph", text: "Visits rose." },
      { type: "chart", chart: "c_one", board: "b_1" },
      { type: "list", items: ["North", "South"] },
      { type: "chart", chart: "c_hidden", board: "b_2" },
      { type: "link", text: "Method", url: "https://example.com/m" },
    ],
  },
  charts: {
    c_one: { spec: { id: "c_one", title: "Visits by month", kind: "bar", source: "ui" }, cell: { columns: [], rows: [] }, board: "b_1", boardName: "Sales" },
    c_hidden: { unavailable: "This chart is not available." },
  },
  reporting: { timeZone: "Asia/Jakarta", weekStart: "monday", numberLocale: "en-US" },
  render: { requestedBy: "Rina", issuedAt: Date.UTC(2026, 10, 13, 7, 5) / 1000, paper: "a4", orientation: "portrait" },
}

describe("the report's print view", () => {
  it("posts the token with no session and prints every block in its order, light", async () => {
    const calls = serve(payload)
    const { container } = render(<ReportRenderView token="a.b.c" />)
    expect(await screen.findByRole("heading", { name: "Q3 visitors", level: 1 })).toBeTruthy()
    expect(calls[0].url).toBe("/api/render/data")
    expect(JSON.parse(calls[0].body)).toEqual({ token: "a.b.c" })
    const blocks = container.querySelector("[data-report-blocks]")!
    // BI-62: a chart's card sits in its width box (`.rd-chart-block`).
    const order = Array.from(blocks.children).map((el) =>
      el.querySelector("[data-report-chart]")?.getAttribute("data-report-chart") ?? el.tagName.toLowerCase(),
    )
    expect(order).toEqual(["h2", "p", "c_one", "ul", "unavailable", "p"])
    expect(screen.getByTestId("tile-c_one").getAttribute("data-dark")).toBe("false")
    expect(screen.getByText("This chart is not available.")).toBeTruthy()
    // A link is printed with its address, since nobody can click a PDF's link reliably.
    expect(screen.getByText("(https://example.com/m)")).toBeTruthy()
    expect(screen.getByText(/Downloaded by Rina, .*14:05/)).toBeTruthy()
    await waitFor(() => expect(flag()).toBe(true))
  })

  // PR #528 review BLOCKER: an uploaded map's boundary is read through the render token, not `apiFetch`.
  it("gives every chart a map loader that reads the boundary through the render token", async () => {
    const mapPayload = {
      ...payload,
      report: { ...payload.report, blocks: [{ type: "chart", chart: "c_map", board: "b_1" }] },
      charts: {
        c_map: { spec: { id: "c_map", title: "By province", kind: "geomap", source: "ui" }, cell: { columns: [], rows: [] }, board: "b_1", boardName: "Sales" },
      },
    }
    const calls = serve(mapPayload)
    render(<ReportRenderView token="a.b.c" />)
    await screen.findByTestId("tile-c_map")
    expect(loaders.length).toBe(1)
    const loader = loaders[0]
    expect(loader).toBeDefined()
    await loader!("m_0123456789ab")
    const last = calls[calls.length - 1]
    expect(last.url).toBe("/api/render/maps/m_0123456789ab/geojson")
    expect(JSON.parse(last.body)).toEqual({ token: "a.b.c" })
    const init = (global.fetch as unknown as { mock: { calls: [unknown, RequestInit][] } }).mock.calls.at(-1)![1]
    expect(init.method).toBe("POST")
  })

  it("a refused token shows the sentence and fails the conversion", async () => {
    let raised = ""
    renderFailure.raise = (m: string) => { raised = m }
    serve({ error: "render token is invalid or expired" }, 401)
    render(<ReportRenderView token="bad" />)
    expect((await screen.findByRole("alert")).textContent).toBe("render token is invalid or expired")
    expect(raised).toBe("render data refused")
  })
})
