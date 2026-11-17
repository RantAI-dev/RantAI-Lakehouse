import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import * as realHttp from "@/services/http"

// PR #246 review BLOCKER 1: a public link and an embed read an uploaded map's
// boundary through their own token, never through `apiFetch` — whose 401
// sends the browser to the login page, which an anonymous viewer has no use
// for. The mocked `apiFetch` records every call, so the test fails if the
// anonymous path ever reaches it.
const apiFetchCalls: string[] = []
const http = { ...realHttp }
mock.module("@/services/http", () => ({
  ...http,
  apiFetch: (input: RequestInfo | URL) => {
    apiFetchCalls.push(String(input))
    return Promise.resolve(new Response("{}", { status: 401 }))
  },
}))
afterAll(() => {
  mock.module("@/services/http", () => http)
})

// The chart needs a canvas; what is handed to it does not.
const drawn: { option: unknown }[] = []
mock.module("./echart", () => ({
  EChart: (props: { option: never }) => {
    drawn.push(props)
    return <div data-testid="chart" />
  },
}))

const { PublicDashboard } = await import("./public-dashboard")
const { EmbedView } = await import("./embed-view")

const originalFetch = global.fetch
const requested: string[] = []

const GEOJSON = {
  type: "FeatureCollection",
  features: [
    { type: "Feature", properties: { name: "North" }, geometry: { type: "Polygon", coordinates: [[[0, 0], [1, 0], [1, 1], [0, 0]]] } },
  ],
}

/** A board with one region chart on the uploaded map `mapId`. */
function payload(mapId: string) {
  return {
    board: { id: "b1", name: "Board" },
    layout: {},
    charts: [{ id: "c1", title: "Regions", kind: "geomap", map: mapId, x: "region", y: "n", source: "ui" }],
    results: { c1: { columns: ["region", "n"], rows: [{ region: "North", n: 1 }] } },
    reporting: { timeZone: "Asia/Jakarta", weekStart: "monday" },
  }
}

/** Serves the board's data and the map boundary; `boundary` decides the file's answer. */
function serve(mapId: string, boundary = 200) {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = String(input)
    requested.push(url)
    if (url.includes("/geojson")) {
      return boundary === 200
        ? new Response(JSON.stringify(GEOJSON), { status: 200 })
        : new Response("not found", { status: boundary })
    }
    return new Response(JSON.stringify(payload(mapId)), { status: 200 })
  }) as unknown as typeof fetch
}

beforeEach(() => {
  apiFetchCalls.length = 0
  requested.length = 0
  drawn.length = 0
  // happy-dom starts on `about:blank`; the logo in the public header is a
  // `next/image`, which needs a real URL to build its `srcSet`.
  ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(
    "http://localhost/",
  )
})
afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

describe("a public link reads an uploaded map through its own token", () => {
  it("asks the link's own route for the boundary and never reaches apiFetch", async () => {
    serve("m_aa11bb22cc33")
    render(<PublicDashboard token="tok" />)
    await waitFor(() => expect(drawn.length).toBeGreaterThan(0))
    expect(requested).toContain("/api/public/dashboard/tok/maps/m_aa11bb22cc33/geojson")
    expect(apiFetchCalls).toEqual([])
  })

  it("says what happened when the boundary cannot be read, and does not redirect", async () => {
    serve("m_bb22cc33dd44", 404)
    render(<PublicDashboard token="tok" />)
    expect(await screen.findByText("This map's boundary file could not be loaded.")).toBeTruthy()
    expect(apiFetchCalls).toEqual([])
  })
})

describe("an embed reads an uploaded map through its own token", () => {
  it("posts the signed token to the embed's own route and never reaches apiFetch", async () => {
    serve("m_cc33dd44ee55")
    render(<EmbedView jwt="a.b.c" />)
    await waitFor(() => expect(drawn.length).toBeGreaterThan(0))
    expect(requested).toContain("/api/embed/maps/m_cc33dd44ee55/geojson")
    expect(apiFetchCalls).toEqual([])
  })

  it("reads the boundary through the public link's route when the embed was opened with a token", async () => {
    serve("m_dd44ee55ff66")
    render(<EmbedView token="tok" />)
    await waitFor(() => expect(drawn.length).toBeGreaterThan(0))
    expect(requested).toContain("/api/public/dashboard/tok/maps/m_dd44ee55ff66/geojson")
    expect(apiFetchCalls).toEqual([])
  })
})
