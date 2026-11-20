// The Catalog Lineage tab's map as a reader uses it (BI-71): the charts that
// read the asset come grouped by dashboard, a group opens in place, and a
// failed read of the viewer's charts or dashboards leaves the map as it was.
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"

import { LineageMindmap, mindmapModel, useChartPlacement } from "./asset-lineage-map"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
})
const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const TABLE = { id: "table:serving.mart_wisman", label: "serving.mart_wisman", kind: "focus" }
const CHARTS = [
  { id: "chart:c1", label: "Visitors by port", kind: "chart" },
  { id: "chart:c2", label: "Visitors by month", kind: "chart" },
  { id: "chart:c3", label: "Arrivals share", kind: "chart" },
]
const MODEL = mindmapModel({
  graph: {
    focus: "serving.mart_wisman",
    nodes: [TABLE, ...CHARTS],
    edges: CHARTS.map((c, i) => ({ id: `e${i}`, from: TABLE.id, to: c.id, kind: "chart" })),
    columnMappings: [],
    supported: true,
  },
  self: { label: "serving.mart_wisman", kind: "gold" },
  dependents: [],
  relatedUpstream: [],
  relatedDownstream: [],
})

/** The map as the Lineage tab mounts it: the placement read first, then the map. */
function Tab() {
  const { loading, placement } = useChartPlacement(true)
  if (loading) return <p>Loading</p>
  return <LineageMindmap model={MODEL} placement={placement} />
}

function serve(specs: () => Response) {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const path = String(input)
    if (path.startsWith("/api/dashboard/specs")) return specs()
    if (path.startsWith("/api/dashboard/boards")) {
      return json({ boards: [{ id: "b_qa", name: "QA Auto" }, { id: "b_arr", name: "Arrivals" }] })
    }
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch
}

const groupNames = () =>
  screen.getAllByRole("button", { name: /· \d+ charts?$/ }).map((b) => b.getAttribute("aria-label"))

describe("LineageMindmap groups (BI-71)", () => {
  it("draws one node per dashboard, and opens and closes a group in place", async () => {
    serve(() =>
      json({ charts: [{ id: "c1", board: "b_qa" }, { id: "c2", board: "b_qa" }, { id: "c3", board: "b_arr" }] })
    )
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
    serve(() => json({ error: "The charts could not be loaded" }, 500))
    render(<Tab />)
    await waitFor(() => expect(screen.queryByText("Loading")).toBeNull())
    expect(screen.getByText("Visitors by port")).toBeTruthy()
    expect(screen.getByText("Arrivals share")).toBeTruthy()
    expect(screen.queryByRole("button", { name: /charts?$/ })).toBeNull()
    // And says nothing about it: grouping is a reading aid.
    expect(screen.queryByText(/could not be loaded/)).toBeNull()
  })
})
