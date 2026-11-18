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
/** The `dark` each chart was drawn with, so a test can see what the print view told it. */
const darks: boolean[] = []
let captured: RenderMode | null = null
/** The map loader each chart was given. */
const loaders: (((id: string) => Promise<Response>) | undefined)[] = []
mock.module("./tile-body", () => ({
  ...tileBody,
  TileBody: function Body({ spec, dark, loadMap }: { spec: { id: string }; dark?: boolean; loadMap?: (id: string) => Promise<Response> }) {
    const mode = useRenderMode()
    React.useEffect(() => {
      if (spec.id !== "c_chart" || !mode) return
      captured = mode
      mode.chartMounted()
    }, [mode, spec.id])
    rendered.push(spec.id)
    loaders.push(loadMap)
    darks.push(dark === true)
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
const { RenderView, NOT_IN_PDF, renderFailure } = await import("./render-view")

const originalFetch = global.fetch
// `quietImages` changes the window's address and the image-loading setting, which every file of the run shares
// (one happy-dom window per `bun test` process); both are put back after each test.
const happyDom = () => (window as unknown as { happyDOM?: { settings: Record<string, unknown>; setURL: (u: string) => void } }).happyDOM
const originalUrl = window.location.href
const originalImageLoading = happyDom()?.settings.disableImageLoading
const flag = () => (window as unknown as { __lakehouseRenderReady?: boolean }).__lakehouseRenderReady

beforeEach(() => {
  delete (window as unknown as { __lakehouseRenderReady?: boolean }).__lakehouseRenderReady
  captured = null
})
afterEach(() => {
  cleanup()
  rendered.length = 0
  loaders.length = 0
  darks.length = 0
  global.fetch = originalFetch
  delete (window as unknown as { __lakehouseRenderReady?: boolean }).__lakehouseRenderReady
  const dom = happyDom()
  if (dom) {
    dom.settings.disableImageLoading = originalImageLoading
    dom.setURL(originalUrl)
  }
})

// happy-dom would load (and fail) a real picture the moment it is mounted, which would fire `error`
// before the test does; the tests fire `load` and `error` themselves.
function quietImages() {
  const dom = happyDom()
  if (dom) {
    dom.settings.disableImageLoading = true
    dom.setURL("http://localhost/")
  }
}

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

  // QA LH-BI-1-01 F2 (BI-53, D15): a dark dashboard printed as a dark box on
  // the white page left white bands.
  it("a dark dashboard is printed light: the page, the cards and the charts", async () => {
    serve(payload({ theme: "dark" }))
    const { container } = render(<RenderView token="t" />)
    await screen.findByRole("heading", { name: "Visits" })
    const view = container.querySelector("[data-render-view]")
    expect(view).not.toBeNull()
    expect(view!.className).toContain("theme-light")
    expect(view!.className).not.toContain("dark")
    await waitFor(() => expect(darks.length).toBeGreaterThan(0))
    expect(darks.every((d) => d === false)).toBe(true)
  })

  it("a chart image keeps the dashboard's dark theme", async () => {
    serve(payload({ theme: "dark" }))
    const { container } = render(<RenderView token="t" chart />)
    await screen.findByRole("heading", { name: "Note" })
    const box = container.querySelector("[data-render-chart]")
    expect(box).not.toBeNull()
    expect(box!.className).toContain("dark")
    await waitFor(() => expect(darks.length).toBeGreaterThan(0))
    expect(darks.every((d) => d === true)).toBe(true)
  })

  // BI-53 (D17): the print view is as wide as the paper's printable width, so
  // a chart is created in the layout that is printed.
  it("the print view is as wide as the paper's printable width", async () => {
    serve(payload())
    const { container, unmount } = render(<RenderView token="t" />)
    await screen.findByRole("heading", { name: "Visits" })
    const view = container.querySelector("[data-render-view]") as HTMLElement
    expect(view.style.width).toBe("697px")
    unmount()
    serve(payload({ render: { paper: "a4", orientation: "landscape" } }))
    const second = render(<RenderView token="t" />)
    await screen.findByRole("heading", { name: "Visits" })
    const landscape = second.container.querySelector("[data-render-view]") as HTMLElement
    expect(landscape.style.width).toBe("1027px")
  })

  // BI-36: the map's boundary is read through the render token, not through the
  // signed-in loader, which a print view with no session cannot pass.
  it("gives every chart a map loader that reads the boundary through the render token", async () => {
    const calls = serve(payload())
    render(<RenderView token="a.b.c" />)
    await waitFor(() => expect(loaders.length).toBeGreaterThan(0))
    const loader = loaders.find((l) => l !== undefined)
    expect(loader).toBeDefined()
    await loader!("m_0123456789ab")
    const last = calls[calls.length - 1]
    expect(last.url).toBe("/api/render/maps/m_0123456789ab/geojson")
    expect(JSON.parse(last.body)).toEqual({ token: "a.b.c" })
    const init = (global.fetch as unknown as { mock: { calls: [unknown, RequestInit][] } }).mock.calls.at(-1)![1]
    expect(init.method).toBe("POST")
  })

  // BI-42 (plan D6b): an image card's picture comes through the console's own
  // origin; the view waits for it, and the placeholder stands in when it fails.
  it("points an image card at the render image route with the token, and waits for the picture", async () => {
    const charts = [
      { id: "c_img", title: "Logo", kind: "image", source: "ui", def: { url: "https://img.example/logo.png" } },
      { id: "c_web", title: "Status", kind: "web", source: "ui", def: { url: "https://img.example/status" } },
    ]
    quietImages()
    serve(payload({ charts, layout: { c_img: { x: 0, y: 0, w: 6, h: 3, tab: "t1" }, c_web: { x: 6, y: 0, w: 6, h: 3, tab: "t1" } } }))
    const { container } = render(<RenderView token="a.b.c" />)
    const img = (await waitFor(() => {
      const found = container.querySelector("img")
      expect(found).not.toBeNull()
      return found
    })) as HTMLImageElement
    expect(img.getAttribute("src")).toBe(
      `/api/render/image?token=${encodeURIComponent("a.b.c")}&src=${encodeURIComponent("https://img.example/logo.png")}`,
    )
    expect(img.getAttribute("loading")).toBe("eager")
    // The web card keeps the placeholder and mounts no frame.
    expect(screen.getAllByText(NOT_IN_PDF).length).toBe(1)
    expect(container.querySelector("iframe")).toBeNull()
    // Not ready while the picture is loading, ready after it has.
    await new Promise((r) => setTimeout(r, 100))
    expect(flag()).toBeUndefined()
    act(() => { img.dispatchEvent(new Event("load")) })
    await waitFor(() => expect(flag()).toBe(true))
  })

  it("prints the placeholder for an image card whose picture fails, and is ready", async () => {
    quietImages()
    const charts = [{ id: "c_img", title: "Logo", kind: "image", source: "ui", def: { url: "https://img.example/logo.png" } }]
    serve(payload({ charts, layout: { c_img: { x: 0, y: 0, w: 6, h: 3, tab: "t1" } } }))
    const { container } = render(<RenderView token="t" />)
    const img = (await waitFor(() => {
      const found = container.querySelector("img")
      expect(found).not.toBeNull()
      return found
    })) as HTMLImageElement
    act(() => { img.dispatchEvent(new Event("error")) })
    await screen.findByText(NOT_IN_PDF)
    expect(container.querySelector("img")).toBeNull()
    await waitFor(() => expect(flag()).toBe(true))
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

  it("shows the server's sentence when the token is refused, raises a failure so the conversion fails, and is ready so Gotenberg checks at once (R3)", async () => {
    const raised: string[] = []
    const real = renderFailure.raise
    renderFailure.raise = (m) => { raised.push(m) }
    try {
      serve({ error: "render token is invalid or expired" }, 401)
      render(<RenderView token="x" />)
      expect((await screen.findByRole("alert")).textContent).toBe("render token is invalid or expired")
      await waitFor(() => expect(flag()).toBe(true))
      expect(raised).toEqual(["render data refused"])
    } finally {
      renderFailure.raise = real
    }
  })

  it("raises no failure when the data loads", async () => {
    const raised: string[] = []
    const real = renderFailure.raise
    renderFailure.raise = (m) => { raised.push(m) }
    try {
      serve(payload())
      render(<RenderView token="t" />)
      await screen.findByRole("heading", { name: "Visits" })
      expect(raised).toEqual([])
    } finally {
      renderFailure.raise = real
    }
  })
})

describe("the chart image view (BI-19 part D, plan D13)", () => {
  it("draws the one chart under its title and the applied filters in the box Gotenberg screenshots", async () => {
    serve(payload({
      layout: { c_chart: { x: 0, y: 0, w: 12, h: 4 } },
      tabs: undefined,
      charts: [{ id: "c_chart", title: "Visits by region", kind: "bar", mart: "m", x: "k", y: "v", source: "ui" }],
    }))
    const { container } = render(<RenderView token="a.b.c" chart />)
    expect(await screen.findByRole("heading", { name: "Visits by region" })).toBeTruthy()
    expect(container.querySelector("[data-render-chart]")).not.toBeNull()
    expect(screen.getByTestId("render-filters").textContent).toContain("region: north, south")
    expect(screen.getByTestId("tile-c_chart")).toBeTruthy()
    // No dashboard chrome: no tab headings, no "Downloaded by" block.
    expect(screen.queryByText("Downloaded by")).toBeNull()
    await waitFor(() => expect(captured).not.toBeNull())
    act(() => captured?.chartFinished())
    await waitFor(() => expect(flag()).toBe(true))
  })
})
