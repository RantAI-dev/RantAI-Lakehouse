import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { KpiCard } from "./kpi-card"
import { PivotTable } from "./pivot-table"
import { RawTable, type RawTableSpec } from "./raw-table"
import { TileBody } from "./tile-body"
import type { Rows } from "./tile-dialogs"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { TableDefFields } from "@/lib/table-types"
import { withPreviewDef } from "@/lib/preview-spec"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

const SPEC: RawTableSpec = { mart: "mart_x", title: "Visits", def: { tableMode: "rows", columns: ["place", "visitors"], sortColumn: "visitors", sortDir: "desc" } }
const FIRST_PAGE: Rows = {
  columns: ["place", "visitors"],
  rows: [{ place: "Ubud", visitors: 9 }, { place: "Kuta", visitors: 7 }],
  total: 120, limit: 50, offset: 0,
}

/** Serves a page of `{ place }` rows for any records request and records every URL. */
function stubRecords(total = 120): URL[] {
  const urls: URL[] = []
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), "http://console.example")
    urls.push(url)
    const offset = Number(url.searchParams.get("offset") ?? 0)
    const columns = (url.searchParams.get("columns") ?? "").split(",")
    const rows = [{ [columns[0]]: `row-${offset + 1}` }]
    return json({ columns, rows, total, limit: 50, offset, filtersSkipped: [] })
  }) as unknown as typeof fetch
  return urls
}

describe("RawTable", () => {
  it("shows the first page the dashboard carried, without asking the server", () => {
    const urls = stubRecords()
    render(<RawTable spec={SPEC} cell={FIRST_PAGE} paging={{ filters: [] }} />)
    expect(screen.getByText("Ubud")).toBeTruthy()
    expect(screen.getByText("1–2 of 120")).toBeTruthy()
    expect(urls).toHaveLength(0)
  })

  it("pages through the records route with its columns, its saved sort and the dashboard's filters", async () => {
    const urls = stubRecords()
    render(<RawTable spec={SPEC} cell={FIRST_PAGE} paging={{ filters: [{ column: "kab", values: ["a"] }] }} />)

    fireEvent.click(screen.getByRole("button", { name: "Next page" }))
    await screen.findByText("row-51")
    const q = urls[0].searchParams
    expect(urls[0].pathname).toBe("/api/dashboard/records")
    expect(q.get("columns")).toBe("place,visitors")
    expect(q.get("sortColumn")).toBe("visitors")
    expect(q.get("sortDir")).toBe("desc")
    expect(q.get("offset")).toBe("50")
    expect(q.get("mart")).toBe("mart_x")
    expect(JSON.parse(q.get("filters") ?? "[]")).toHaveLength(1)
    expect(q.has("column")).toBe(false)
  })

  it("sorts by a header across the whole result and returns to the first page", async () => {
    const urls = stubRecords()
    render(<RawTable spec={SPEC} cell={FIRST_PAGE} paging={{ filters: [] }} />)

    fireEvent.click(screen.getByRole("button", { name: "Next page" }))
    await screen.findByText("row-51")
    fireEvent.click(screen.getByRole("button", { name: "place" }))
    await waitFor(() => expect(urls.at(-1)?.searchParams.get("sortColumn")).toBe("place"))
    expect(urls.at(-1)?.searchParams.get("sortDir")).toBe("asc")
    expect(urls.at(-1)?.searchParams.get("offset")).toBe("0")
    await screen.findByText("row-1")
    fireEvent.click(screen.getByRole("button", { name: "place" }))
    await waitFor(() => expect(urls.at(-1)?.searchParams.get("sortDir")).toBe("desc"))
  })

  it("lists only the columns that are not hidden, under their labels, in their order", () => {
    const spec: RawTableSpec = { ...SPEC, def: { tableMode: "rows", columns: ["visitors", "place", "secret"], columnSettings: { secret: { hidden: true }, place: { label: "Where" } } } }
    const cell: Rows = { columns: ["visitors", "place", "secret"], rows: [{ visitors: 1, place: "Ubud", secret: "x" }] }
    render(<RawTable spec={spec} cell={cell} />)
    const heads = screen.getAllByRole("columnheader").map((h) => h.textContent)
    expect(heads).toEqual(["visitors", "Where"])
    expect(screen.queryByText("x")).toBeNull()
  })

  it("is the first page only where there is no paging, and says how many rows there are", () => {
    render(<RawTable spec={SPEC} cell={FIRST_PAGE} />)
    expect(screen.getByText("First 2 of 120 rows")).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Next page" })).toBeNull()
    // No header is a button to sort by.
    expect(screen.queryByRole("button", { name: "place" })).toBeNull()
  })

  it("offers no paging, and does not claim a total, when the count was not read", () => {
    const noTotal: Rows = { ...FIRST_PAGE, total: undefined }
    render(<RawTable spec={SPEC} cell={noTotal} paging={{ filters: [] }} />)
    expect(screen.getByText("Ubud")).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Next page" })).toBeNull()
    expect(screen.queryByText(/of 120/)).toBeNull()
  })

  it("shows the server's sentence and reference when a page cannot be read", async () => {
    global.fetch = mock(async () => json({ error: "The dashboard query failed. Reference: ab12cd34" }, 422)) as unknown as typeof fetch
    render(<RawTable spec={SPEC} cell={FIRST_PAGE} paging={{ filters: [] }} />)
    fireEvent.click(screen.getByRole("button", { name: "Next page" }))
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("The dashboard query failed."))
    expect(screen.getByText("ab12cd34")).toBeTruthy()
  })
})

describe("cells", () => {
  const spec: RawTableSpec = { mart: "m", title: "t", def: { tableMode: "rows", columns: ["u"], columnSettings: { u: { format: "link" } } } }

  it("renders an anchor that opens in a new tab only for an http or https value", () => {
    const cell: Rows = { columns: ["u"], rows: [{ u: "https://example.com/a" }, { u: "javascript:alert(1)" }, { u: "ftp://x" }] }
    render(<RawTable spec={spec} cell={cell} />)
    const links = screen.getAllByRole("link")
    expect(links).toHaveLength(1)
    expect(links[0].getAttribute("href")).toBe("https://example.com/a")
    expect(links[0].getAttribute("target")).toBe("_blank")
    expect(links[0].getAttribute("rel")).toBe("noopener noreferrer")
    expect(screen.getByText("javascript:alert(1)").closest("a")).toBeNull()
  })

  it("renders an image only for an https value, with no referrer", () => {
    const imageSpec: RawTableSpec = { ...spec, def: { ...spec.def, columnSettings: { u: { format: "image" } } } }
    const cell: Rows = { columns: ["u"], rows: [{ u: "https://example.com/a.png" }, { u: "http://example.com/b.png" }] }
    const { container } = render(<RawTable spec={imageSpec} cell={cell} />)
    const imgs = container.querySelectorAll("img")
    expect(imgs).toHaveLength(1)
    expect(imgs[0].getAttribute("referrerpolicy")).toBe("no-referrer")
    expect(imgs[0].getAttribute("loading")).toBe("lazy")
    expect(screen.getByText("http://example.com/b.png")).toBeTruthy()
  })
})

describe("PivotTable", () => {
  const def: TableDefFields = {
    rows: ["province", "kind"], columns: ["month"], values: [{ column: "visitors", aggregate: "sum" }], totals: "all",
  }
  const r = (province: string, kind: string, month: string, v: number, g: [number, number, number]) => ({ province, kind, month, __v0: v, __g0: g[0], __g1: g[1], __g2: g[2] })
  const cell: Rows = {
    columns: [],
    rows: [
      r("Bali", "Hotel", "2026-01", 1, [0, 0, 0]), r("Bali", "Villa", "2026-01", 2, [0, 0, 0]),
      r("Bali", "", "2026-01", 3, [0, 1, 0]), r("Bali", "Hotel", "x", 1, [0, 0, 1]), r("Bali", "Villa", "x", 2, [0, 0, 1]),
      r("Bali", "", "x", 3, [0, 1, 1]), r("", "", "2026-01", 3, [1, 1, 0]), r("", "", "x", 3, [1, 1, 1]),
    ],
  }

  it("lays out the groups, the subtotal and the grand total from the database's cells", () => {
    render(<PivotTable def={def} cell={cell} />)
    const rows = screen.getAllByRole("row").map((tr) => Array.from(tr.children).map((c) => c.textContent))
    expect(rows[0]).toEqual(["province", "kind", "2026-01", "Total"])
    const body = rows.slice(1)
    expect(body[0]).toEqual(["Bali", "Hotel", "1", "1"])
    expect(body[1]).toEqual(["", "Villa", "2", "2"])
    expect(body[2].slice(-2)).toEqual(["3", "3"])
    expect(body.at(-1)).toEqual(["Total", "3", "3"])
  })

  it("folds a group to its subtotal and unfolds it again", () => {
    render(<PivotTable def={def} cell={cell} />)
    fireEvent.click(screen.getByRole("button", { name: "Collapse Bali" }))
    expect(screen.queryByText("Hotel")).toBeNull()
    expect(screen.getByRole("button", { name: "Expand Bali" }).getAttribute("aria-expanded")).toBe("false")
    fireEvent.click(screen.getByRole("button", { name: "Expand Bali" }))
    expect(screen.getByText("Hotel")).toBeTruthy()
  })

  it("cannot fold without subtotals", () => {
    render(<PivotTable def={{ ...def, totals: "grand" }} cell={cell} />)
    expect(screen.queryByRole("button", { name: /Collapse/ })).toBeNull()
  })

  it("says it was cut at the cell cap", () => {
    render(<PivotTable def={def} cell={{ ...cell, truncated: true }} />)
    expect(screen.getByText(/Cut off: only the first 10,000 cells/)).toBeTruthy()
  })

  it("formats a value by its column's setting", () => {
    const withFormat: TableDefFields = { ...def, columnSettings: { visitors: { format: "currency" } } }
    render(<PivotTable def={withFormat} cell={cell} />)
    expect(screen.getAllByText(/^Rp/).length).toBeGreaterThan(0)
  })
})

describe("KpiCard", () => {
  it("shows the change as an amount and a percent with an arrow, a trend line and the periods compared", () => {
    const cell: Rows = {
      columns: ["day", "v"], grain: "month",
      rows: [{ day: "2026-01-01", v: "80" }, { day: "2026-02-01", v: "100" }, { day: "2026-03-01", v: "150" }],
    }
    const def: TableDefFields = { compare: { kind: "previous", dateColumn: "day", period: "month" } }
    const { container } = render(<KpiCard def={def} cell={cell} caption="visits" />)
    expect(screen.getByText("150")).toBeTruthy()
    expect(screen.getByText(/\+50/)).toBeTruthy()
    expect(screen.getByText(/\+50%/)).toBeTruthy()
    expect(screen.getByText(/vs Feb 2026/)).toBeTruthy()
    expect(container.querySelector("polyline")).not.toBeNull()
    expect(container.querySelector(".text-emerald-600")).not.toBeNull()
  })

  it("colours a rise as worse when down is good", () => {
    const cell: Rows = { columns: [], grain: "month", rows: [{ day: "2026-01-01", v: 10 }, { day: "2026-02-01", v: 12 }] }
    const { container } = render(<KpiCard def={{ compare: { kind: "previous", dateColumn: "day", period: "month" }, goodDirection: "down" }} cell={cell} />)
    expect(container.querySelector(".text-red-600")).not.toBeNull()
    expect(container.querySelector(".text-emerald-600")).toBeNull()
  })

  it("shows the amount only when the previous value is zero", () => {
    const cell: Rows = { columns: [], grain: "month", rows: [{ day: "2026-01-01", v: 0 }, { day: "2026-02-01", v: 5 }] }
    render(<KpiCard def={{ compare: { kind: "previous", dateColumn: "day", period: "month" } }} cell={cell} />)
    expect(screen.getByText(/\+5/)).toBeTruthy()
    expect(screen.queryByText(/%/)).toBeNull()
  })

  it("shows the distance to a goal and the percent of it", () => {
    render(<KpiCard def={{ compare: { kind: "goal", value: 200 } }} cell={{ columns: ["v"], rows: [{ v: "150" }] }} />)
    expect(screen.getByText(/75% of goal/)).toBeTruthy()
    expect(screen.getByText(/-50 to go/)).toBeTruthy()
  })

  it("says there is nothing to compare with when only one period has data", () => {
    render(<KpiCard def={{ compare: { kind: "previous", dateColumn: "day", period: "month" } }} cell={{ columns: [], grain: "month", rows: [{ day: "2026-03-01", v: 4 }] }} />)
    expect(screen.getByText("No earlier period to compare with.")).toBeTruthy()
  })
})

describe("TileBody", () => {
  const spec = (kind: ChartRenderSpec["kind"], def: unknown): ChartRenderSpec => ({ id: "t", title: "T", kind, mart: "m", x: "", y: "", source: "ui", def })

  it("keeps a KPI without a comparison as the plain number", () => {
    render(<TileBody spec={spec("kpi", {})} cell={{ columns: ["v"], rows: [{ v: 1234 }] }} dark={false} loading={false} />)
    expect(screen.getByText("1,234")).toBeTruthy()
  })

  it("gives a grouped table its column labels, formats and hidden columns", () => {
    const def = { columnSettings: { b: { hidden: true }, a: { label: "Visitors", format: "currency" } } }
    render(<TileBody spec={spec("table", def)} cell={{ columns: ["a", "b"], rows: [{ a: 1500, b: "no" }] }} dark={false} loading={false} />)
    expect(screen.getByText("Visitors")).toBeTruthy()
    expect(screen.queryByText("b")).toBeNull()
    expect(screen.getByText(/^Rp/)).toBeTruthy()
  })

  it("draws a raw table from a stored definition, and a pivot too", () => {
    render(<TileBody spec={spec("table", SPEC.def)} cell={FIRST_PAGE} dark={false} loading={false} />)
    expect(screen.getByText("Ubud")).toBeTruthy()
    cleanup()
    render(<TileBody spec={spec("pivot", { rows: ["p"], values: [{ column: "v", aggregate: "sum" }] })} cell={{ columns: [], rows: [{ p: "a", __v0: 5, __g0: 0 }] }} dark={false} loading={false} />)
    expect(screen.getByText("a")).toBeTruthy()
  })
})

describe("builder preview (BI-16A review fix R2)", () => {
  // The preview route's spec has no `def`; the builder attaches its payload.
  const served = { id: "p", title: "T", kind: "table", mart: "m", x: "", y: "", source: "ui" } as ChartRenderSpec
  const cell: Rows = { columns: ["a", "b"], rows: [{ a: 1500, b: "hidden-value" }] }
  const payload = { tableMode: "rows", columns: ["a", "b"], columnSettings: { a: { format: "currency" }, b: { hidden: true } } }

  it("shows a raw table's currency and hidden column only once the definition is attached", () => {
    render(<TileBody spec={{ ...served, kind: "table", def: { tableMode: "rows", columns: ["a", "b"] } }} cell={cell} dark={false} loading={false} />)
    expect(screen.getByText("hidden-value")).toBeTruthy()
    cleanup()
    render(<TileBody spec={withPreviewDef(served, payload)} cell={cell} dark={false} loading={false} />)
    expect(screen.queryByText("hidden-value")).toBeNull()
    expect(screen.getByText(/^Rp/)).toBeTruthy()
  })

  it("draws a KPI's comparison from the attached payload", () => {
    const kpi = { ...served, kind: "kpi" } as ChartRenderSpec
    const rows: Rows = { columns: ["v"], rows: [{ v: 150 }] }
    render(<TileBody spec={kpi} cell={rows} dark={false} loading={false} />)
    expect(screen.queryByText(/of goal/)).toBeNull()
    cleanup()
    render(<TileBody spec={withPreviewDef(kpi, { compare: { kind: "goal", value: 200 } })} cell={rows} dark={false} loading={false} />)
    expect(screen.getByText(/75% of goal/)).toBeTruthy()
  })
})

describe("pivot labels and folding (BI-16A review fix R3, R5)", () => {
  it("labels a grained date field with BI-9's bucket label, not the bucket start", () => {
    const def: TableDefFields = { rows: ["p"], columns: ["q"], values: [{ column: "v", aggregate: "sum" }], totals: "none" }
    const cell: Rows = {
      columns: [], grain: "quarter",
      rows: [{ p: "a", q: "2025-07-01", __v0: 1, __g0: 0, __g1: 0 }, { p: "a", q: "2025-10-01", __v0: 2, __g0: 0, __g1: 0 }],
    }
    render(<PivotTable def={def} cell={cell} />)
    expect(screen.getByText("Q3 2025")).toBeTruthy()
    expect(screen.getByText("Q4 2025")).toBeTruthy()
    expect(screen.queryByText("2025-07-01")).toBeNull()
  })

  it("has nothing to fold with one row field: its groups are leaves (folding needs two row fields)", () => {
    const def: TableDefFields = { rows: ["p"], values: [{ column: "v", aggregate: "sum" }], totals: "all" }
    const cell: Rows = { columns: [], rows: [{ p: "a", __v0: 1, __g0: 0 }, { p: "", __v0: 1, __g0: 1 }] }
    render(<PivotTable def={def} cell={cell} />)
    expect(screen.queryByRole("button", { name: /Collapse/ })).toBeNull()
  })

  it("makes every sticky cell opaque and stacked so labels cannot draw over values", () => {
    const def: TableDefFields = { rows: ["p"], columns: ["m"], values: [{ column: "v", aggregate: "sum" }], totals: "grand" }
    const cell: Rows = { columns: [], rows: [{ p: "a", m: "x", __v0: 1, __g0: 0, __g1: 0 }, { p: "", m: "", __v0: 1, __g0: 1, __g1: 1 }] }
    const { container } = render(<PivotTable def={def} cell={cell} />)
    const sticky = Array.from(container.querySelectorAll(".sticky"))
    expect(sticky.length).toBeGreaterThan(0)
    for (const el of sticky) {
      expect(/\bbg-(card|muted)\b/.test(el.className)).toBe(true)
      expect(/\bz-(10|20|30)\b/.test(el.className)).toBe(true)
    }
    // The corner is above both axes.
    expect(container.querySelector("thead th.left-0")?.className).toContain("z-30")
  })
})
