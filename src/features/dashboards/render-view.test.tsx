import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import * as realTileBody from "./tile-body"
import * as realGrid from "./dashboard-grid"
import { useRenderMode, type RenderMode } from "./render-mode"

// BI-1 plan D4 / feature decisions 3 and 4. `mock.module` outlives this file in `bun test src`, so the
// real exports are kept and put back in `afterAll` (as in dashboard-look.test.tsx).
const tileBody = { ...realTileBody }
const grid = { ...realGrid }
const rendered: string[] = []
let captured: RenderMode | null = null
mock.module("./tile-body", () => ({
  ...tileBody,
  TileBody: function Body({ spec }: { spec: { id: string } }) {
    const mode = useRenderMode()
    React.useEffect(() => {
      if (spec.id !== "c_chart" || !mode) return
      captured = mode
      mode.chartMounted()
    }, [mode, spec.id])
    rendered.push(spec.id)
    return <div data-testid={`tile-${spec.id}`} />
  },
}))
mock.module("./dashboard-grid", () => ({
  ...grid,
  DashboardGrid: ({ items, layout, tab }: { items: { id: string; body: React.ReactNode }[]; layout: Record<string, { tab?: string }>; tab?: string }) => (
    <div data-grid-tab={tab ?? "all"}>
      {items.filter((i) => !tab || layout[i.id]?.tab === tab).map((i) => <div key={i.id} data-testid={`item-${i.id}`}>{i.body}</div>)}
    </div>
  ),
}))
afterAll(() => {
  mock.module("./tile-body", () => tileBody)
  mock.module("./dashboard-grid", () => grid)
})

import * as React from "react"
const { RenderView, NOT_IN_PDF } = await import("./render-view")

const originalFetch = global.fetch
const flag = () => (window as unknown as { __lakehouseRenderReady?: boolean }).__lakehouseRenderReady

beforeEach(() => {
  delete (window as unknown as { __lakehouseRenderReady?: boolean }).__lakehouseRenderReady
  captured = null
})
afterEach(() => {
  cleanup()
  rendered.length = 0
  global.fetch = originalFetch
})

function payload(extra: Record<string, unknown> = {}) {
  return {
    board: "b1",
    boards: [{ id: "b1", name: "Visits" }],
    layout: { c_text: { x: 0, y: 0, w: 6, h: 3, tab: "t1" }, c_img: { x: 0, y: 0, w: 6, h: 3, tab: "t2" }, c_chart: { x: 6, y: 0, w: 6, h: 3, tab: "t2" } },
    tabs: [{ id: "t1", title: "First" }, { id: "t2", title: "Second" }],
    filters: [{ column: "region", values: ["north", "south"] }],
    params: [{ param: { id: "1", name: "top", label: "Top N", type: "number", default: 5 }, value: 10 }],
    charts: [
      { id: "c_text", title: "Note", kind: "text", source: "ui", text: "hello" },
      { id: "c_img", title: "Logo", kind: "image", source: "ui" },
      { id: "c_chart", title: "Chart", kind: "bar", mart: "m", x: "k", y: "v", source: "ui" },
    ],
    results: {},
    reporting: { timeZone: "Asia/Jakarta", weekStart: "monday", numberLocale: "en-US" },
    render: { requestedBy: "Rina", issuedAt: Date.UTC(2026, 9, 17, 7, 5) / 1000, paper: "a4", orientation: "portrait" },
    ...extra,
  }
}

function serve(body: unknown, status = 200) {
  const calls: { url: string; body: string }[] = []
  global.fetch = mock(async (url: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(url), body: String(init?.body ?? "") })
    return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
  }) as unknown as typeof fetch
  return calls
}

describe("the PDF print view", () => {
  it("posts the token with no session, and lists the board, filters, parameters and who asked", async () => {
    const calls = serve(payload())
    render(<RenderView token="a.b.c" />)
    expect(await screen.findByRole("heading", { name: "Visits" })).toBeTruthy()
    expect(calls[0].url).toBe("/api/render/data")
    expect(JSON.parse(calls[0].body)).toEqual({ token: "a.b.c" })
    expect(screen.getByTestId("render-filters").textContent).toBe("region: north, south")
    expect(screen.getByTestId("render-params").textContent).toBe("Top N: 10")
    // 07:05 UTC is 14:05 in the report time zone.
    expect(screen.getByText(/Rina, .*14:05/)).toBeTruthy()
  })

  it("draws every tab with its title, each after the first starting a new page", async () => {
    serve(payload())
    const { container } = render(<RenderView token="t" />)
    await screen.findByRole("heading", { name: "First" })
    expect(screen.getByRole("heading", { name: "Second" })).toBeTruthy()
    const sections = container.querySelectorAll("[data-render-tab]")
    expect(sections.length).toBe(2)
    expect(sections[0].className).not.toContain("render-tab-break")
    expect(sections[1].className).toContain("render-tab-break")
    expect(screen.getByTestId("item-c_text")).toBeTruthy()
    expect(screen.getByTestId("item-c_chart")).toBeTruthy()
  })

  it("prints an image or web-page card as a placeholder and never mounts it", async () => {
    serve(payload())
    render(<RenderView token="t" />)
    await screen.findByText(NOT_IN_PDF)
    expect(NOT_IN_PDF).toBe("Shown in the console only")
    expect(rendered).not.toContain("c_img")
    expect(rendered).toContain("c_text")
  })

  it("says ready only after the data is in and every chart has finished drawing", async () => {
    serve(payload())
    render(<RenderView token="t" />)
    await waitFor(() => expect(captured).not.toBeNull())
    // The chart has mounted and not finished: not ready, after more than a frame.
    await new Promise((r) => setTimeout(r, 100))
    expect(flag()).toBeUndefined()
    act(() => captured?.chartFinished())
    await waitFor(() => expect(flag()).toBe(true))
  })

  it("is ready at once when the board has no chart", async () => {
    serve(payload({ charts: [{ id: "c_text", title: "Note", kind: "text", source: "ui", text: "x" }], tabs: undefined }))
    render(<RenderView token="t" />)
    await waitFor(() => expect(flag()).toBe(true))
  })

  it("shows the server's sentence when the token is refused, and is ready so the PDF holds the sentence", async () => {
    serve({ error: "render token is invalid or expired" }, 401)
    render(<RenderView token="x" />)
    expect((await screen.findByRole("alert")).textContent).toBe("render token is invalid or expired")
    await waitFor(() => expect(flag()).toBe(true))
  })
})
