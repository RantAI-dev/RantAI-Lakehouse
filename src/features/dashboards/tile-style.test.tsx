import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { CondRule } from "@/lib/chart-style"
import { KpiCard } from "./kpi-card"
import { PivotTable } from "./pivot-table"
import { RawTable } from "./raw-table"
import { ReportingProvider } from "./reporting-context"
import { TileBody } from "./tile-body"
import type { Rows } from "./tile-dialogs"

afterEach(cleanup)

const below1000: CondRule = { column: "visitors", rule: { type: "compare", op: "lt", value: 1000 }, color: "#ff0000" }
const spec = (kind: ChartRenderSpec["kind"], def: unknown): ChartRenderSpec => ({ id: "t", title: "T", kind, mart: "m", x: "", y: "", source: "ui", def })
/** A style colour as `#rrggbb`: the DOM may hand back either `#rrggbb` or `rgb(r, g, b)`. */
function hex(css: string): string {
  const m = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(css)
  return m ? `#${[m[1], m[2], m[3]].map((n) => Number(n).toString(16).padStart(2, "0")).join("")}` : css
}
const bg = (el: HTMLElement) => hex(el.style.backgroundColor)
const fg = (el: HTMLElement) => hex(el.style.color)

describe("conditional colours on each tile kind", () => {
  it("a grouped table colours the cells below 1,000 red and leaves the others", () => {
    const def = { conditional: [below1000] }
    render(<TileBody spec={spec("table", def)} cell={{ columns: ["place", "visitors"], rows: [{ place: "Ubud", visitors: 500 }, { place: "Kuta", visitors: 2000 }] }} dark={false} loading={false} />)
    expect(bg(screen.getByText("500").closest("td")!)).toBe("#ff0000")
    expect(bg(screen.getByText("2,000").closest("td")!)).toBe("")
  })

  it("a raw table colours its cells by the rule of their column", () => {
    const cell: Rows = { columns: ["place", "visitors"], rows: [{ place: "Ubud", visitors: 500 }, { place: "Kuta", visitors: 2000 }], total: 2, limit: 50, offset: 0 }
    render(<RawTable spec={{ mart: "m", title: "T", def: { tableMode: "rows", columns: ["place", "visitors"], conditional: [below1000] } }} cell={cell} />)
    expect(bg(screen.getByText("500").closest("td")!)).toBe("#ff0000")
    expect(bg(screen.getByText("2,000").closest("td")!)).toBe("")
  })

  it("a pivot colours a cell by the rule of its value column", () => {
    const r = (v: number, g: [number, number]) => ({ province: "Bali", __v0: v, __g0: g[0], __g1: g[1] })
    const cell: Rows = { columns: [], rows: [r(500, [0, 1]), r(500, [1, 1])] }
    render(<PivotTable def={{ rows: ["province"], values: [{ column: "visitors", aggregate: "sum" }], totals: "all", conditional: [below1000] }} cell={cell} />)
    const cells = screen.getAllByText("500").map((e) => e.closest("td")!)
    expect(cells.length).toBeGreaterThan(0)
    expect(cells.every((c) => bg(c) === "#ff0000")).toBe(true)
  })

  it("a plain KPI turns green above its goal and stays plain below it", () => {
    const goal: CondRule = { column: "visitors", rule: { type: "compare", op: "gt", value: 1000 }, color: "#16a34a", target: "text" }
    const { rerender } = render(<TileBody spec={spec("kpi", { conditional: [goal] })} cell={{ columns: ["v"], rows: [{ v: 1500 }] }} dark={false} loading={false} />)
    expect(fg(screen.getByText("1,500"))).toBe("#16a34a")
    rerender(<TileBody spec={spec("kpi", { conditional: [goal] })} cell={{ columns: ["v"], rows: [{ v: 900 }] }} dark={false} loading={false} />)
    expect(fg(screen.getByText("900"))).toBe("")
  })

  it("a KPI with a comparison is coloured too", () => {
    const goal: CondRule = { column: "visitors", rule: { type: "compare", op: "gt", value: 1000 }, color: "#16a34a", target: "text" }
    const cell: Rows = { columns: ["day", "v"], grain: "month", rows: [{ day: "2026-01-01", v: "80" }, { day: "2026-02-01", v: "1500" }] }
    render(<KpiCard def={{ compare: { kind: "previous", dateColumn: "day", period: "month" }, conditional: [goal] }} cell={cell} />)
    expect(fg(screen.getByText("1,500"))).toBe("#16a34a")
  })
})

describe("number formats on tiles", () => {
  it("a KPI shows the chart's format in the deployment's locale", () => {
    const def = { format: { style: "compact", currency: "IDR", decimals: 2 } }
    render(
      <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday", numberLocale: "id-ID" }}>
        <TileBody spec={spec("kpi", def)} cell={{ columns: ["v"], rows: [{ v: 1234567.891 }] }} dark={false} loading={false} />
      </ReportingProvider>,
    )
    expect(screen.getByText("Rp 1,23 M")).toBeTruthy()
  })

  it("a table cell takes the chart's format unless its column has a format of its own", () => {
    const def = { format: { style: "number", decimals: 1, suffix: " pcs" }, columnSettings: { b: { format: "currency", currency: "USD", decimals: 0 } } }
    render(<TileBody spec={spec("table", def)} cell={{ columns: ["a", "b"], rows: [{ a: 3, b: 1500 }] }} dark={false} loading={false} />)
    expect(screen.getByText("3.0 pcs")).toBeTruthy()
    expect(screen.getByText("$1,500")).toBeTruthy()
  })

  it("a metric's format hint is the default format of its value", () => {
    render(<TileBody spec={spec("table", {})} cell={{ columns: ["rate"], rows: [{ rate: 0.125 }], metricFormats: { rate: "percent" } }} dark={false} loading={false} />)
    expect(screen.getByText("12.5%")).toBeTruthy()
  })

  it("a currency column with no code stays rupiah under the English locale", () => {
    render(<TileBody spec={spec("table", { columnSettings: { a: { format: "currency" } } })} cell={{ columns: ["a"], rows: [{ a: 1500 }] }} dark={false} loading={false} />)
    expect(screen.getByText(/^Rp/)).toBeTruthy()
  })
})

describe("a chart with a logarithmic axis", () => {
  it("says so instead of drawing a wrong axis when a value is zero", () => {
    const chart = { ...spec("bar", { axes: { y: { log: true } } }), x: "k", y: "v" }
    render(<TileBody spec={chart} cell={{ columns: ["k", "v"], rows: [{ k: "a", v: 5 }, { k: "b", v: 0 }] }} dark={false} loading={false} />)
    expect(screen.getByRole("status").textContent).toBe("A logarithmic axis cannot show zero or negative values.")
  })
})
