import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { MapsAdmin } from "./maps-page"

const originalFetch = global.fetch
afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
const MAP = { id: "m_0123456789ab", name: "Sales regions", nameProperty: "REGION", regionCount: 12, createdAt: "2026-10-24T00:00:00Z" }
type Call = { url: string; method: string; body: BodyInit | null | undefined }

function mockApi(handlers: Partial<Record<string, () => Response>> = {}): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = (init?.method ?? "GET").toUpperCase()
    calls.push({ url: String(input), method, body: init?.body })
    return (handlers[`${method} ${String(input)}`] ?? (() => (method === "GET" ? json({ maps: [MAP] }) : json(MAP))))()
  }) as unknown as typeof fetch
  return calls
}

const GEOJSON = '{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"id":1,"REGION":"North"},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}}]}'
const file = (name = "regions.geojson", text = GEOJSON) => new File([text], name, { type: "application/geo+json" })

describe("Admin > Maps (BI-16 C2b)", () => {
  it("lists the uploaded maps with their region count and name property", async () => {
    mockApi()
    render(<MapsAdmin canWrite />)
    expect(await screen.findByText("Sales regions")).toBeTruthy()
    expect(screen.getByText(/12 regions, names from “REGION”/)).toBeTruthy()
  })

  it("offers no upload, rename or delete without settings:write", async () => {
    mockApi()
    render(<MapsAdmin canWrite={false} />)
    await screen.findByText("Sales regions")
    expect(screen.queryByLabelText("GeoJSON file")).toBeNull()
    expect(screen.queryByRole("button", { name: "Delete Sales regions" })).toBeNull()
    expect(screen.getByText("Only an administrator can upload or change maps.")).toBeTruthy()
  })

  it("reads the first feature's properties, offers them as the name property, and uploads the file with the choice", async () => {
    const calls = mockApi()
    render(<MapsAdmin canWrite />)
    await screen.findByText("Sales regions")
    fireEvent.change(screen.getByLabelText("GeoJSON file"), { target: { files: [file()] } })
    expect((await screen.findByLabelText("Name") as HTMLInputElement).value).toBe("regions")
    expect(screen.getByText("North")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Upload map" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true))
    const post = calls.find((c) => c.method === "POST")!
    expect(post.url).toBe("/api/maps")
    const form = post.body as FormData
    expect(form.get("name")).toBe("regions")
    expect(form.get("nameProperty")).toBe("REGION")
    expect((form.get("file") as File).name).toBe("regions.geojson")
    expect(await screen.findByText("Sales regions uploaded: 12 regions.")).toBeTruthy()
  })

  it("refuses a file over 50 MB without reading or sending it", async () => {
    const calls = mockApi()
    render(<MapsAdmin canWrite />)
    await screen.findByText("Sales regions")
    const big = file()
    Object.defineProperty(big, "size", { value: 50 * 1024 * 1024 + 1 })
    fireEvent.change(screen.getByLabelText("GeoJSON file"), { target: { files: [big] } })
    expect((await screen.findByRole("alert")).textContent).toBe("This file is over 50 MB. Simplify it or split it, then upload again.")
    expect(calls.some((c) => c.method === "POST")).toBe(false)
  })

  it("shows the server's sentence when it refuses a file", async () => {
    mockApi({ "POST /api/maps": () => json({ error: "Two regions are both called North." }, 400) })
    render(<MapsAdmin canWrite />)
    await screen.findByText("Sales regions")
    fireEvent.change(screen.getByLabelText("GeoJSON file"), { target: { files: [file()] } })
    await screen.findByLabelText("Name")
    fireEvent.click(screen.getByRole("button", { name: "Upload map" }))
    expect((await screen.findByRole("alert")).textContent).toBe("Two regions are both called North.")
  })

  it("renames a map", async () => {
    const calls = mockApi()
    render(<MapsAdmin canWrite />)
    fireEvent.click(await screen.findByRole("button", { name: "Rename Sales regions" }))
    fireEvent.change(screen.getByLabelText("New name for Sales regions"), { target: { value: "Regions 2026" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PATCH")).toBe(true))
    const patch = calls.find((c) => c.method === "PATCH")!
    expect(patch.url).toBe("/api/maps/m_0123456789ab")
    expect(JSON.parse(String(patch.body))).toEqual({ name: "Regions 2026" })
  })

  it("lists the charts that use a map when its delete is refused", async () => {
    mockApi({
      "DELETE /api/maps/m_0123456789ab": () =>
        json({ error: "This map is used by 1 chart.", charts: [{ chartId: "c1", chartTitle: "Sales by region", dashboardId: "d1", dashboardTitle: "Sales" }] }, 409),
    })
    render(<MapsAdmin canWrite />)
    fireEvent.click(await screen.findByRole("button", { name: "Delete Sales regions" }))
    fireEvent.click(await screen.findByRole("button", { name: "Delete map" }))
    expect(await screen.findByText("This map is used by 1 chart.")).toBeTruthy()
    expect(screen.getByRole("list", { name: "Charts that use this map" }).textContent).toContain("Sales by region")
  })

  it("says it could not load when the server has no maps route", async () => {
    global.fetch = mock(async () => json({ error: "Not found" }, 404)) as unknown as typeof fetch
    render(<MapsAdmin canWrite />)
    expect(await screen.findByRole("button", { name: /retry|try again/i })).toBeTruthy()
  })
})
