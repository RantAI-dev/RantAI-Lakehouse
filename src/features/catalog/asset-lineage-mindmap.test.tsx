// The Catalog Lineage tab's map as a reader uses it (BI-71): the charts that
// read the asset come grouped by dashboard, a group opens in place, and a
// failed read of the viewer's charts or dashboards leaves the map as it was;
// the map opens fitted to its box and pans and zooms inside it.
import { Profiler } from "react"
import { act, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test"

import { boxHeight, fitScale } from "@/lib/pan-zoom"
import { stubRectWidth, stubResizeObserver } from "@/lib/test-layout"
import { dashboardService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import { ServiceError } from "@/services/errors"
import { GraphCard, type AssetLineageState } from "./asset-lineage"
import { LineageMindmap, mindmapLayout, mindmapModel, useChartPlacement } from "./asset-lineage-map"

// The two reads behind the map are stubbed on the service itself, in each test
// and restored after it: a test must not depend on the network, or on what
// another file left on `fetch` or on the HTTP layer in a shared bun process.
const spies: { mockRestore: () => void }[] = []
afterEach(() => {
  while (spies.length) spies.pop()!.mockRestore()
})

const TABLE = { id: "table:serving.mart_wisman", label: "serving.mart_wisman", kind: "focus" }
const CHARTS = [
  { id: "chart:c1", label: "Visitors by port", kind: "chart" },
  { id: "chart:c2", label: "Visitors by month", kind: "chart" },
  { id: "chart:c3", label: "Arrivals share", kind: "chart" },
]
const GRAPH_WITH_CHARTS = {
  focus: "serving.mart_wisman",
  nodes: [TABLE, ...CHARTS],
  edges: CHARTS.map((c, i) => ({ id: `e${i}`, from: TABLE.id, to: c.id, kind: "chart" })),
  columnMappings: [],
  supported: true,
}
const MODEL = mindmapModel({
  graph: GRAPH_WITH_CHARTS,
  self: { label: "serving.mart_wisman", kind: "gold" },
  dependents: [],
  relatedUpstream: [],
  relatedDownstream: [],
})
const ASSET: AssetDetail = {
  id: "mart-wisman",
  name: "Mart Wisman",
  namespace: "serving",
  type: "table",
  layer: "gold",
  tier: "hot",
  classification: "internal",
  owner: "dagster",
  domain: "tourism",
  description: "",
  format: "ClickHouse",
  engine: "hot-store",
  rows: 0,
  sizeBytes: 0,
  columnCount: 0,
  freshnessLagSeconds: 0,
  lastUpdated: "2026-10-20T00:00:00Z",
  health: "unknown",
  residency: "id-jakarta",
  schema: [],
  sample: [],
  qualityChecks: [],
  policySummary: [],
  usage: null,
  recentQueries: [],
  dependents: [],
  changeHistory: [],
  snapshots: [],
  schemaVersions: [],
  upstream: [],
  downstream: [],
  tableKey: "serving.mart_wisman",
}

/** The map as the Lineage tab mounts it: the placement read first, then the map. */
function Tab() {
  const { loading, placement } = useChartPlacement(true)
  if (loading) return <p>Loading</p>
  return <LineageMindmap model={MODEL} placement={placement} />
}

const BOARDS = [{ id: "b_qa", name: "QA Auto" }, { id: "b_arr", name: "Arrivals" }]

/** Answers the boards read, and the charts read as the test says: a list, or a failure. */
function serve(outlines: { id: string; board: string }[] | Error) {
  spies.push(
    spyOn(dashboardService, "listChartOutlines").mockImplementation((async () => {
      if (outlines instanceof Error) throw outlines
      return outlines.map((o) => ({ ...o, kind: "" }))
    }) as unknown as typeof dashboardService.listChartOutlines),
    spyOn(dashboardService, "listBoards").mockResolvedValue(BOARDS as never)
  )
}

const groupNames = () =>
  screen.getAllByRole("button", { name: /· \d+ charts?$/ }).map((b) => b.getAttribute("aria-label"))

describe("LineageMindmap groups (BI-71)", () => {
  it("draws one node per dashboard, and opens and closes a group in place", async () => {
    serve([{ id: "c1", board: "b_qa" }, { id: "c2", board: "b_qa" }, { id: "c3", board: "b_arr" }])
    render(<Tab />)
    const qa = await screen.findByRole("button", { name: "QA Auto · 2 charts" })
    expect(groupNames()).toEqual(["QA Auto · 2 charts", "Arrivals · 1 chart"])
    expect(qa.getAttribute("aria-expanded")).toBe("false")
    expect(screen.queryByText("Visitors by port")).toBeNull()

    fireEvent.click(qa)
    expect(qa.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getByText("Visitors by port")).toBeTruthy()
    expect(screen.getByText("Visitors by month")).toBeTruthy()
    expect(screen.queryByText("Arrivals share")).toBeNull()

    // One group open at a time: opening another closes the first.
    fireEvent.click(screen.getByRole("button", { name: "Arrivals · 1 chart" }))
    expect(screen.queryByText("Visitors by port")).toBeNull()
    expect(screen.getByText("Arrivals share")).toBeTruthy()

    // A second click closes it.
    fireEvent.click(screen.getByRole("button", { name: "Arrivals · 1 chart" }))
    expect(screen.queryByText("Arrivals share")).toBeNull()
  })

  it("keeps every chart as its own node when the charts cannot be read", async () => {
    serve(new ServiceError("unavailable", "The charts could not be loaded"))
    render(<Tab />)
    // `findBy` rides a MutationObserver like the first test's does, and needs no
    // timer: a `waitFor` poll waits on one, which a shared process can leave stopped.
    expect(await screen.findByText("Visitors by port")).toBeTruthy()
    expect(screen.queryByText("Loading")).toBeNull()
    expect(screen.getByText("Arrivals share")).toBeTruthy()
    expect(screen.queryByRole("button", { name: /charts?$/ })).toBeNull()
    // And says nothing about it: grouping is a reading aid.
    expect(screen.queryByText(/could not be loaded/)).toBeNull()
  })

  // PR #642 review 1: assetLineage resolves after mount, flipping `charted` from
  // false to true; GraphCard must keep its skeleton until placement loads.
  it("never draws the ungrouped map when lineage arrives with charts after the first render", async () => {
    serve([{ id: "c1", board: "b_qa" }, { id: "c2", board: "b_qa" }, { id: "c3", board: "b_arr" }])
    const loading: AssetLineageState = { status: "loading", data: null, error: null, reload: () => {} }
    const loaded: AssetLineageState = { status: "success", data: GRAPH_WITH_CHARTS, error: null, reload: () => {} }
    let sawUngrouped = false
    const onRender = () => {
      if (document.body.querySelector('[aria-label="Lineage graph"]') && !document.body.querySelector('[aria-expanded]')) {
        sawUngrouped = true
      }
    }
    const { rerender } = render(
      <Profiler id="graph-card" onRender={onRender}>
        <GraphCard a={ASSET} state={loading} />
      </Profiler>
    )
    // Let useChartPlacement(false) settle its initial no-op read before lineage resolves.
    await act(async () => {
      await Promise.resolve()
    })
    rerender(
      <Profiler id="graph-card" onRender={onRender}>
        <GraphCard a={ASSET} state={loaded} />
      </Profiler>
    )
    expect(await screen.findByRole("button", { name: "QA Auto · 2 charts" })).toBeTruthy()
    expect(sawUngrouped).toBe(false)
    expect(screen.queryByText("Visitors by port")).toBeNull()
  })

  it("falls back to the ungrouped map when the placement read fails after lineage arrives", async () => {
    serve(new ServiceError("unavailable", "The charts could not be loaded"))
    const loading: AssetLineageState = { status: "loading", data: null, error: null, reload: () => {} }
    const loaded: AssetLineageState = { status: "success", data: GRAPH_WITH_CHARTS, error: null, reload: () => {} }
    const { rerender } = render(<GraphCard a={ASSET} state={loading} />)
    await act(async () => {
      await Promise.resolve()
    })
    rerender(<GraphCard a={ASSET} state={loaded} />)
    expect(await screen.findByText("Visitors by port")).toBeTruthy()
    expect(screen.getByText("Arrivals share")).toBeTruthy()
    expect(screen.queryByRole("button", { name: /charts?$/ })).toBeNull()
    expect(screen.queryByText(/could not be loaded/)).toBeNull()
  })
})

// BI-71 decision 3: the map pans and zooms inside its box and opens fitted.
describe("LineageMindmap pan and zoom (BI-71)", () => {
  // A chain ten columns long: 2712 px wide at zoom 1, far wider than the box.
  const CHAIN = Array.from({ length: 10 }, (_, i) => ({ id: `table:silver.t${i}`, label: `silver.t${i}`, kind: i === 0 ? "focus" : "silver" }))
  const WIDE = mindmapModel({
    graph: {
      focus: "silver.t0",
      nodes: CHAIN,
      edges: CHAIN.slice(1).map((n, i) => ({ id: `e${i}`, from: CHAIN[i].id, to: n.id, kind: "view" })),
      columnMappings: [],
      supported: true,
    },
    self: { label: "silver.t0", kind: "silver" },
    dependents: [],
    relatedUpstream: [],
    relatedDownstream: [],
  })
  const BOX_WIDTH = 848
  let undo: (() => void)[] = []
  beforeEach(() => {
    undo.push(stubResizeObserver(), stubRectWidth(() => BOX_WIDTH))
  })
  afterEach(() => {
    for (const u of undo.reverse()) u()
    undo = []
  })

  const content = () => {
    const { width, height } = mindmapLayout(WIDE)
    return { width, height }
  }
  const fit = () => {
    const c = content()
    return fitScale(c, { width: BOX_WIDTH, height: boxHeight(c, BOX_WIDTH, 24, 560) }, 24)
  }
  const map = () => screen.getByRole("region", { name: "Lineage map" })
  const zoom = () => Number(map().getAttribute("data-zoom"))
  const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement

  it("opens fitted to its box, with nothing left to zoom out to", () => {
    render(<LineageMindmap model={WIDE} />)
    expect(fit()).toBeLessThan(1)
    expect(zoom()).toBeCloseTo(fit(), 3)
    expect(button("Zoom out").disabled).toBe(true)
    expect(button("Fit").disabled).toBe(true)
    expect(button("Zoom in").disabled).toBe(false)
  })

  it("zooms in by the buttons up to 2, back out to the fit, and fits again", () => {
    render(<LineageMindmap model={WIDE} />)
    fireEvent.click(button("Zoom in"))
    expect(zoom()).toBeCloseTo(fit() * 1.25, 3)
    expect(button("Fit").disabled).toBe(false)
    for (let i = 0; i < 20; i++) if (!button("Zoom in").disabled) fireEvent.click(button("Zoom in"))
    expect(zoom()).toBe(2)
    expect(button("Zoom in").disabled).toBe(true)
    for (let i = 0; i < 20; i++) if (!button("Zoom out").disabled) fireEvent.click(button("Zoom out"))
    expect(zoom()).toBeCloseTo(fit(), 3)
    fireEvent.click(button("Zoom in"))
    fireEvent.click(button("Fit"))
    expect(zoom()).toBeCloseTo(fit(), 3)
    expect(button("Fit").disabled).toBe(true)
  })

  it("zooms with + - 0 while it has focus, and with Ctrl + wheel, but leaves a plain wheel to the page", () => {
    render(<LineageMindmap model={WIDE} />)
    fireEvent.keyDown(map(), { key: "+" })
    expect(zoom()).toBeCloseTo(fit() * 1.25, 3)
    fireEvent.keyDown(map(), { key: "-" })
    expect(zoom()).toBeCloseTo(fit(), 3)
    fireEvent.keyDown(map(), { key: "+" })
    fireEvent.keyDown(map(), { key: "0" })
    expect(zoom()).toBeCloseTo(fit(), 3)

    const plain = new WheelEvent("wheel", { deltaY: -100, bubbles: true, cancelable: true })
    map().dispatchEvent(plain)
    expect(plain.defaultPrevented).toBe(false)
    expect(zoom()).toBeCloseTo(fit(), 3)

    const pinch = new WheelEvent("wheel", { deltaY: -50, bubbles: true, cancelable: true })
    // happy-dom's WheelEvent drops the modifier keys of its init dict.
    Object.defineProperty(pinch, "ctrlKey", { value: true })
    act(() => {
      map().dispatchEvent(pinch)
    })
    expect(pinch.defaultPrevented).toBe(true)
    expect(zoom()).toBeGreaterThan(fit())
  })

  it("pans by dragging, and a drag that ends on a node does not open it", () => {
    render(<LineageMindmap model={WIDE} />)
    fireEvent.click(button("Zoom in"))
    fireEvent.click(button("Zoom in"))
    const drawing = map().querySelector("[data-slot=map-drawing]") as HTMLElement
    const before = drawing.style.transform
    const link = screen.getByText("t3").closest("a") as HTMLAnchorElement
    fireEvent.pointerDown(link, { pointerId: 1, button: 0, pointerType: "mouse", clientX: 400, clientY: 30 })
    fireEvent.pointerMove(link, { pointerId: 1, pointerType: "mouse", clientX: 300, clientY: 30 })
    fireEvent.pointerUp(link, { pointerId: 1, pointerType: "mouse", clientX: 300, clientY: 30 })
    expect(drawing.style.transform).not.toBe(before)
    const click = new MouseEvent("click", { bubbles: true, cancelable: true })
    link.dispatchEvent(click)
    expect(click.defaultPrevented).toBe(true)
    // A plain press and release is still a click.
    fireEvent.pointerDown(link, { pointerId: 2, button: 0, pointerType: "mouse", clientX: 300, clientY: 30 })
    fireEvent.pointerUp(link, { pointerId: 2, pointerType: "mouse", clientX: 300, clientY: 30 })
    const again = new MouseEvent("click", { bubbles: true, cancelable: true })
    link.dispatchEvent(again)
    expect(again.defaultPrevented).toBe(false)
  })

  // Owner QA round 1: a full-screen view of the same map.
  describe("full screen", () => {
    beforeEach(() => {
      undo.push(stubRectWidth(() => 1200, 800))
    })
    // The overlay fades for 200 ms before it unmounts; wait that out inside act.
    const settle = () => act(async () => { await new Promise((r) => setTimeout(r, 260)) })
    const dialog = () => screen.queryByRole("dialog", { name: "Lineage map, full screen" })

    it("opens in an overlay over the window, fitted, and closes by the button, Esc or F, returning focus", async () => {
      render(<LineageMindmap model={WIDE} />)
      fireEvent.click(button("Full screen"))
      expect(dialog()).not.toBeNull()
      expect(document.body.style.overflow).toBe("hidden")
      expect(button("Exit full screen")).toBeDefined()
      // Fitted inside the 1200 x 800 window, not the 560 px inline cap.
      const c = content()
      expect(Number(screen.getByRole("region", { name: "Lineage map" }).getAttribute("data-zoom"))).toBeCloseTo(
        fitScale(c, { width: 1200, height: 800 }, 24),
        3
      )

      fireEvent.click(button("Exit full screen"))
      await settle()
      expect(dialog()).toBeNull()
      expect(document.body.style.overflow).not.toBe("hidden")
      expect(document.activeElement).toBe(button("Full screen"))

      fireEvent.click(button("Full screen"))
      fireEvent.keyDown(document, { key: "Escape" })
      await settle()
      expect(dialog()).toBeNull()

      fireEvent.keyDown(map(), { key: "f" })
      expect(dialog()).not.toBeNull()
      fireEvent.keyDown(document, { key: "F" })
      await settle()
      expect(dialog()).toBeNull()
    })

    // PR #642 review SHOULD-FIX 1: Tab stays inside the overlay.
    it("keeps Tab and Shift+Tab inside the overlay", () => {
      render(<LineageMindmap model={WIDE} />)
      fireEvent.click(button("Full screen"))
      const items = Array.from(
        dialog()!.querySelectorAll<HTMLElement>('a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"])')
      )
      expect(items.length).toBeGreaterThan(1)
      const first = items[0]
      const last = items[items.length - 1]
      last.focus()
      const tab = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true })
      last.dispatchEvent(tab)
      expect(tab.defaultPrevented).toBe(true)
      expect(document.activeElement).toBe(first)
      const back = new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true })
      first.dispatchEvent(back)
      expect(back.defaultPrevented).toBe(true)
      expect(document.activeElement).toBe(last)
    })
  })
})
