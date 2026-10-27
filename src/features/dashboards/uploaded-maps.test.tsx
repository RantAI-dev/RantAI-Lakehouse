import { afterEach, describe, expect, it, mock } from "bun:test"
import { ensureMap, mapEntry, mapFeatureNames } from "./echarts-maps"
import { matchRegionRows } from "@/lib/geo-regions"

// BI-16 C2b: an uploaded map is known by its id alone and read through apiFetch.
const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
})

describe("uploaded maps in the catalogue", () => {
  it("finds a server-made id without a list, and rejects ids that are not server-made", () => {
    expect(mapEntry("m_0123456789ab")).toMatchObject({ uploaded: true, level: "uploaded", url: "/api/maps/m_0123456789ab/geojson" })
    expect(mapEntry("m_0123456789a")).toBeUndefined()
    expect(mapEntry("m_0123456789AB")).toBeUndefined()
    expect(mapEntry("../../etc")).toBeUndefined()
  })

  it("loads the file from the API and registers its feature names", async () => {
    const asked: string[] = []
    global.fetch = mock(async (input: RequestInfo | URL) => {
      asked.push(String(input))
      return new Response(JSON.stringify({ type: "FeatureCollection", features: [{ type: "Feature", properties: { name: "North" }, geometry: { type: "Polygon", coordinates: [[[0, 0], [1, 0], [1, 1], [0, 0]]] } }] }), { status: 200 })
    }) as unknown as typeof fetch
    expect(await ensureMap("m_aaaaaaaaaaaa")).toBe(true)
    expect(asked).toEqual(["/api/maps/m_aaaaaaaaaaaa/geojson"])
    expect(mapFeatureNames("m_aaaaaaaaaaaa")).toEqual(["North"])
  })

  it("is not available when the file cannot be read", async () => {
    global.fetch = mock(async () => new Response("{}", { status: 404 })) as unknown as typeof fetch
    expect(await ensureMap("m_bbbbbbbbbbbb")).toBe(false)
  })

  it("matches region names as typed, ignoring case, accents and spacing", () => {
    const m = matchRegionRows([{ name: " north  ", value: 2 }, { name: "Zürich", value: 1 }, { name: "Nowhere", value: 5 }], "uploaded", ["North", "Zurich"])
    expect(m.data).toEqual([{ name: "North", value: 2 }, { name: "Zurich", value: 1 }])
    expect(m.unmatchedRows).toBe(1)
  })
})
