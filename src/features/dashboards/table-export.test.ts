import { afterEach, describe, expect, it, mock } from "bun:test"
import { downloadTableExport, exportNotice, fileNameOf, tableExportQuery } from "./table-export"

const originalFetch = global.fetch
afterEach(() => { global.fetch = originalFetch })

describe("table export", () => {
  it("asks for the tile with the dashboard's filters and the viewer's sort", () => {
    const q = tableExportQuery("c_1", [{ column: "kab", values: ["a"] }], { column: "visitors", dir: "desc" })
    expect(q.get("chart")).toBe("c_1")
    expect(JSON.parse(q.get("filters") ?? "[]")).toHaveLength(1)
    expect(q.get("sortColumn")).toBe("visitors")
    expect(q.get("sortDir")).toBe("desc")
    expect(tableExportQuery("c_1", [], null).toString()).toBe("chart=c_1")
  })

  it("says plainly when the file was cut, and at how many rows", () => {
    expect(exportNotice({ rows: 100000, cut: true, cap: 100000 }).message).toBe("Exported the first 100,000 rows")
    expect(exportNotice({ rows: 100000, cut: true, cap: 100000 }).description).toContain("cut")
    expect(exportNotice({ rows: 237, cut: false, cap: 100000 })).toEqual({ message: "Exported 237 rows" })
  })

  it("reads the file name and the row facts from the response headers", async () => {
    URL.createObjectURL = (() => "blob:x") as typeof URL.createObjectURL
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL
    global.fetch = mock(async () => new Response("a\r\n1\r\n", {
      headers: { "content-disposition": 'attachment; filename="Visits.csv"', "x-export-rows": "1", "x-export-cut": "true", "x-export-cap": "100000" },
    })) as unknown as typeof fetch
    const r = await downloadTableExport("c_1", [])
    expect(r).toEqual({ rows: 1, cut: true, cap: 100000, fileName: "Visits.csv" })
    expect(fileNameOf(null)).toBe("table.csv")
  })

  it("throws the server's sentence, not a made-up one", async () => {
    global.fetch = mock(async () => new Response(JSON.stringify({ error: "The dashboard query failed. Reference: ab12" }), { status: 422 })) as unknown as typeof fetch
    await expect(downloadTableExport("c_1", [])).rejects.toThrow("The dashboard query failed. Reference: ab12")
  })
})
