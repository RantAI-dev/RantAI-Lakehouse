import { afterEach, describe, expect, it, mock } from "bun:test"
import { chartExportQuery, downloadChartData, downloadDashboardPdf, downloadMenuKind, pdfExportQuery } from "./chart-download"

const originalFetch = global.fetch
afterEach(() => { global.fetch = originalFetch })

const filters = [{ column: "region", values: ["north"] }]

describe("the queries a download sends", () => {
  it("sends a chart's file kind, value mode and everything the page read it with", () => {
    const q = chartExportQuery("c1", { format: "xlsx", values: "raw" }, { filters, params: '{"top":5}', grain: "month" }, { column: "visits", dir: "desc" })
    expect(Object.fromEntries(q)).toEqual({
      chart: "c1", format: "xlsx", values: "raw",
      filters: JSON.stringify(filters), params: '{"top":5}', grain: "month",
      sortColumn: "visits", sortDir: "desc",
    })
  })

  it("leaves out what the page did not send", () => {
    const q = chartExportQuery("c1", { format: "csv", values: "formatted" }, { filters: [] })
    expect(Object.fromEntries(q)).toEqual({ chart: "c1", format: "csv", values: "formatted" })
  })

  it("sends a PDF's board, paper, orientation and the same reads", () => {
    const q = pdfExportQuery("b1", { paper: "letter", orientation: "landscape" }, { filters, params: null, grain: null })
    expect(Object.fromEntries(q)).toEqual({ board: "b1", paper: "letter", orientation: "landscape", filters: JSON.stringify(filters) })
  })
})

function serve(response: Response) {
  const urls: string[] = []
  global.fetch = mock(async (url: RequestInfo | URL) => { urls.push(String(url)); return response.clone() }) as unknown as typeof fetch
  return urls
}

describe("downloads", () => {
  it("throws the server's own sentence when a download is refused", async () => {
    serve(new Response(JSON.stringify({ error: "permission_denied: Downloads are switched off for your role." }), { status: 403 }))
    await expect(downloadChartData("c1", { format: "csv", values: "raw" }, { filters: [] })).rejects.toThrow("Downloads are switched off for your role.")
    serve(new Response(JSON.stringify({ error: "The PDF service is not reachable. Reference: abc" }), { status: 503 }))
    await expect(downloadDashboardPdf("b1", { paper: "a4", orientation: "portrait" }, { filters: [] })).rejects.toThrow("The PDF service is not reachable. Reference: abc")
  })

  it("reads the counts of a finished chart download and saves the file", async () => {
    const urls = serve(new Response("a,b\r\n", {
      status: 200,
      headers: { "content-disposition": 'attachment; filename="Visits-first-1000000-rows.csv"', "x-export-rows": "1000000", "x-export-cut": "true", "x-export-cap": "1000000" },
    }))
    const done = await downloadChartData("c1", { format: "csv", values: "raw" }, { filters: [] })
    expect(urls[0]).toStartWith("/api/dashboard/chart-export?chart=c1&format=csv&values=raw")
    expect(done).toEqual({ rows: 1_000_000, cut: true, cap: 1_000_000, fileName: "Visits-first-1000000-rows.csv" })
  })
})

describe("the tile menu's download", () => {
  it("is the server download for a chart, the rows on screen for a built-in tile, and nothing without rows or permission", () => {
    expect(downloadMenuKind({ own: true, hasRows: true }, true)).toBe("data")
    expect(downloadMenuKind({ own: false, hasRows: true }, true)).toBe("csv")
    expect(downloadMenuKind({ own: true, hasRows: false }, true)).toBeNull()
    expect(downloadMenuKind({ own: true, hasRows: true }, false)).toBeNull()
    expect(downloadMenuKind({ own: false, hasRows: true }, false)).toBeNull()
  })
})
