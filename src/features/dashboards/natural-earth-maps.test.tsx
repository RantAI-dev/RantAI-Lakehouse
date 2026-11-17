import { cleanup, render, waitFor } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, mock } from "bun:test"
import { existsSync, readdirSync, readFileSync } from "node:fs"
import { join } from "node:path"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"

// BI-16 C2a (plan D4). The chart needs a canvas; what is handed to it does not.
const drawn: { option: { series?: { data?: { name: string; value: number }[] }[] } }[] = []
mock.module("./echart", () => ({
  EChart: (props: { option: never }) => {
    drawn.push(props)
    return <div data-testid="chart" />
  },
}))
const { GeoChart } = await import("./geo-chart")
const { MAP_CATALOGUE, mapEntry, mapCredit, DEFAULT_CHOROPLETH_MAP, DEFAULT_POINT_MAP, countryMapId } = await import("./echarts-maps")

const PUBLIC = join(import.meta.dir, "../../../public")
const realFile = (url: string) => readFileSync(join(PUBLIC, url), "utf8")

// Serve the app's static files from public/, as Next does, but only the ones asked for.
const requested: string[] = []
beforeAll(() => {
  globalThis.fetch = (async (url: string) => {
    requested.push(url)
    const path = join(PUBLIC, url)
    return existsSync(path)
      ? new Response(realFile(url), { status: 200 })
      : new Response("not found", { status: 404 })
  }) as unknown as typeof fetch
})
afterEach(() => { cleanup(); drawn.length = 0; requested.length = 0 })

const spec = (map: string | undefined, x = "country") =>
  ({ id: "c", title: "t", kind: "geomap", x, y: "n", map, source: "ui" }) as unknown as ChartRenderSpec

const last = () => drawn[drawn.length - 1]?.option.series?.[0]?.data

describe("the catalogue keeps the Indonesian and Jakarta maps exactly as they were", () => {
  it("lists them first, with the same ids, urls, levels and labels", () => {
    expect(MAP_CATALOGUE.slice(0, 3)).toEqual([
      { id: "dki-jakarta", label: "Jakarta — cities", url: "/geo/dki-jakarta.geojson", level: "city" },
      { id: "id-provinces", label: "Indonesia — provinces", url: "/geo/id-provinces.geojson", level: "province" },
      { id: "id-regencies", label: "Indonesia — kabupaten/kota", url: "/geo/id-regencies.geojson", level: "regency" },
    ])
    expect(DEFAULT_CHOROPLETH_MAP).toBe("dki-jakarta")
    expect(DEFAULT_POINT_MAP).toBe("id-provinces")
    expect(mapCredit("id-provinces")).toBe("Boundaries: BPS, OCHA via geoBoundaries (CC BY 3.0 IGO)")
    expect(mapCredit("dki-jakarta")).toBeUndefined()
  })

  it("has no states map for Indonesia: its provinces file stays the default for IDN", () => {
    expect(mapEntry(countryMapId("IDN"))).toBeUndefined()
  })

  it("colours the Jakarta map for a geomap with no map set, as before C2a", async () => {
    render(<GeoChart spec={spec(undefined, "area")} rows={[{ area: "KOTA JAKARTA PUSAT", n: 3 }, { area: "Bekasi", n: 1 }]} dark={false} />)
    await waitFor(() => expect(last()).toBeDefined())
    expect(last()).toEqual([{ name: "Jakarta Pusat", value: 3 }])
    expect(requested).toEqual(["/geo/dki-jakarta.geojson"])
  })

  it("colours the provinces by the old name rules", async () => {
    render(<GeoChart spec={spec("id-provinces", "prov")} rows={[{ prov: "DI Yogyakarta", n: 2 }, { prov: "Provinsi Jawa Barat", n: 5 }]} dark={false} />)
    await waitFor(() => expect(last()).toBeDefined())
    expect(last()).toEqual([
      { name: "Daerah Istimewa Yogyakarta", value: 2 },
      { name: "Jawa Barat", value: 5 },
    ])
  })
})

describe("the shipped Natural Earth files", () => {
  it("has a world map with a name on every country and ISO codes where Natural Earth has them", () => {
    const world = JSON.parse(realFile("/geo/world.geojson")) as { features: { properties: Record<string, string> }[] }
    expect(world.features.length).toBeGreaterThan(200)
    for (const f of world.features) {
      expect(f.properties.name).toBeTruthy()
      // Somaliland, Northern Cyprus and the Siachen Glacier have no code: empty, never invented.
      expect(f.properties.ISO_A2).toMatch(/^([A-Z]{2})?$/)
      expect(f.properties.ISO_A3).toMatch(/^([A-Z]{3})?$/)
    }
    expect(world.features.filter((f) => !f.properties.ISO_A2 && !f.properties.ISO_A3).map((f) => f.properties.name).sort()).toEqual(
      ["N. Cyprus", "Siachen Glacier", "Somaliland"]
    )
  })

  it("has one catalogue entry per country file, and a file per entry", () => {
    const files = readdirSync(join(PUBLIC, "geo/admin1")).filter((f) => f.endsWith(".geojson")).map((f) => f.replace(".geojson", ""))
    const entries = MAP_CATALOGUE.filter((m) => m.level === "subdivision")
    expect(entries.map((m) => m.id).sort()).toEqual(files.map(countryMapId).sort())
    expect(new Set(MAP_CATALOGUE.map((m) => m.id)).size).toBe(MAP_CATALOGUE.length)
    expect(mapEntry(countryMapId("USA"))?.label).toBe("United States of America — states/provinces")
  })

  it("keeps the total size of public/geo under the 15 MB budget", () => {
    let total = 0
    const walk = (dir: string) => {
      for (const e of readdirSync(dir, { withFileTypes: true })) {
        if (e.isDirectory()) walk(join(dir, e.name))
        else total += readFileSync(join(dir, e.name)).length
      }
    }
    walk(join(PUBLIC, "geo"))
    expect(total).toBeLessThan(15 * 1024 * 1024)
  })
})

describe("a world map is coloured by country", () => {
  it("joins names, long names and ISO-2 and ISO-3 codes to the same countries", async () => {
    render(<GeoChart spec={spec("world")} rows={[
      { country: "US", n: 1 },
      { country: "United States", n: 2 },
      { country: "FRA", n: 4 },
      { country: "Cote d'Ivoire", n: 8 },
      { country: "Czech Republic", n: 16 },
      { country: "uk", n: 32 },
      { country: "Atlantis", n: 64 },
    ]} dark={false} />)
    await waitFor(() => expect(last()).toBeDefined())
    const byName = Object.fromEntries((last() ?? []).map((d) => [d.name, d.value]))
    expect(byName).toEqual({
      "United States of America": 3,
      France: 4,
      "Côte d'Ivoire": 8,
      Czechia: 16,
      "United Kingdom": 32,
    })
  })
})

describe("a country map is coloured by state or province", () => {
  it("joins names (without accents) and ISO 3166-2 codes, and loads only that country's file", async () => {
    render(<GeoChart spec={spec(countryMapId("BRA"), "state")} rows={[{ state: "Sao Paulo", n: 1 }, { state: "BR-RJ", n: 2 }, { state: "Nowhere", n: 4 }]} dark={false} />)
    await waitFor(() => expect(last()).toBeDefined())
    const names = (last() ?? []).map((d) => d.name).sort()
    expect(names).toEqual(["Rio de Janeiro", "São Paulo"])
    expect(requested).toEqual(["/geo/admin1/BRA.geojson"])
  })
})
