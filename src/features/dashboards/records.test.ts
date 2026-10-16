import { describe, expect, it } from "bun:test"
import { hasNextPage, pageRange, recordsQuery, splitReference, type RecordsRequest } from "./records"

const base: RecordsRequest = { title: "Bali", mart: "mart_x", column: "region", value: "Bali", filters: [] }

describe("recordsQuery", () => {
  it("names the mart, the column and value, one page and the offset", () => {
    const q = recordsQuery(base, 100)
    expect(q.get("mart")).toBe("mart_x")
    expect(q.get("column")).toBe("region")
    expect(q.get("value")).toBe("Bali")
    expect(q.get("limit")).toBe("50")
    expect(q.get("offset")).toBe("100")
    expect(q.has("filters")).toBe(false)
  })

  it("names a SQL source instead of the mart", () => {
    const q = recordsQuery({ ...base, mart: "", sqlSource: "s_1234abcd" }, 0)
    expect(q.get("sqlSource")).toBe("s_1234abcd")
    expect(q.has("mart")).toBe(false)
  })

  it("sends neither column nor value for a whole tile", () => {
    const q = recordsQuery({ title: "Tile", mart: "mart_x", filters: [] }, 0)
    expect(q.has("column")).toBe(false)
    expect(q.has("value")).toBe(false)
  })

  it("keeps an empty value, which is a real category", () => {
    expect(recordsQuery({ ...base, value: "" }, 0).get("value")).toBe("")
  })

  it("passes the active filters along and leaves out the inert ones", () => {
    const q = recordsQuery({
      ...base,
      filters: [{ column: "kab", values: ["Ubud"] }, { column: "empty", values: [] }],
    }, 0)
    expect(JSON.parse(q.get("filters") ?? "null")).toEqual([{ column: "kab", values: ["Ubud"] }])
  })

  it("encodes a value with a space, an ampersand and non-ASCII text as one parameter", () => {
    const q = recordsQuery({ ...base, value: "a b&c=d/é#" }, 0)
    expect(new URLSearchParams(q.toString()).get("value")).toBe("a b&c=d/é#")
  })
})

describe("paging", () => {
  it("reads 1–50 of N on the first page and a short last page honestly", () => {
    const rows = (n: number) => Array.from({ length: n }, () => ({}))
    expect(pageRange({ rows: rows(50), total: 58, offset: 0 })).toBe("1–50 of 58")
    expect(pageRange({ rows: rows(8), total: 58, offset: 50 })).toBe("51–58 of 58")
    expect(pageRange({ rows: [], total: 0, offset: 0 })).toBe("0 of 0")
  })

  it("has a next page only while rows remain beyond this one", () => {
    const rows = (n: number) => Array.from({ length: n }, () => ({}))
    expect(hasNextPage({ rows: rows(50), total: 58, offset: 0 })).toBe(true)
    expect(hasNextPage({ rows: rows(8), total: 58, offset: 50 })).toBe(false)
    expect(hasNextPage({ rows: rows(50), total: 50, offset: 0 })).toBe(false)
  })
})

describe("splitReference", () => {
  it("separates the server's reference from its sentence", () => {
    expect(splitReference("The dashboard query failed. Reference: ee0f295936"))
      .toEqual({ message: "The dashboard query failed.", errorId: "ee0f295936" })
  })

  it("leaves a message with no reference whole", () => {
    expect(splitReference("column 'x' does not exist")).toEqual({ message: "column 'x' does not exist" })
  })
})

describe("recordsQuery for a question of a combined chart (BI-13)", () => {
  it("names the chart and the question, with that question's own source and column", () => {
    const q = recordsQuery({ title: "Rooms", mart: "mart_rooms", column: "month", value: "2026-02-01", filters: [], chart: "c_1", layer: 0 }, 0)
    expect(q.get("mart")).toBe("mart_rooms")
    expect(q.get("column")).toBe("month")
    expect(q.get("chart")).toBe("c_1")
    expect(q.get("layer")).toBe("0")
  })

  it("sends no layer for the chart's own question, and none without a chart", () => {
    expect(recordsQuery({ ...base, chart: "c_1" }, 0).has("layer")).toBe(false)
    expect(recordsQuery({ ...base, layer: 1 }, 0).has("layer")).toBe(false)
  })
})
