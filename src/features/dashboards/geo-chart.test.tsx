import { act, cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { GeoChart } from "./geo-chart"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"

// Found in BI-19 QA, 2026-10-28: an uploaded map's catalogue entry is a new
// object on every `mapEntry` call, and the load effect depended on it, so the
// tile re-rendered without end and froze the tab.
afterEach(cleanup)

describe("the map tile of an uploaded map", () => {
  it("loads the boundary file a bounded number of times and settles", async () => {
    let loads = 0
    // A refused file keeps ensureMap from caching, so a loop would call the loader again each round.
    const loadMap = async () => {
      loads += 1
      if (loads > 50) throw new Error("render loop")
      return new Response("{}", { status: 404 })
    }
    const spec = { kind: "geomap", map: "m_0123456789ab" } as unknown as ChartRenderSpec
    render(<GeoChart spec={spec} rows={[]} dark={false} loadMap={loadMap} />)
    for (let i = 0; i < 10; i++) await act(async () => { await new Promise((r) => setTimeout(r, 5)) })
    expect(screen.getByText("Map data not loaded")).toBeTruthy()
    expect(loads).toBeLessThan(5)
  })
})
