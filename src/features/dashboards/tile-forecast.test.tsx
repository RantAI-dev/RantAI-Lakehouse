import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import reference from "@/lib/forecast-reference.json"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { Rows } from "./tile-dialogs"

// BI-17 part B, T4. The tile is what both the dashboard and the PDF render
// view mount (`render-view.tsx` renders `TileBody`), so what it hands the chart
// is what the PDF draws. The chart itself needs a canvas.
const drawn: { option: { series?: { name?: string; data?: unknown[] }[]; xAxis?: { data: string[] } } }[] = []
mock.module("./echart", () => ({
  EChart: (props: (typeof drawn)[number]) => {
    drawn.push(props)
    return <div data-testid="chart" />
  },
}))
const { TileBody } = await import("./tile-body")

afterEach(() => { cleanup(); drawn.length = 0 })

const SPEC = { id: "c_1", title: "Passengers", kind: "line", mart: "mart_air", x: "month", y: "passengers", source: "ui" } as unknown as ChartRenderSpec
const pad = (n: number) => String(n).padStart(2, "0")
const cellOf = (n: number, referenceLines: unknown[]): Rows => ({
  columns: ["month", "passengers"],
  grain: "month",
  rows: reference.airline.values.slice(0, n).map((v, i) => ({ month: `${2018 + Math.floor(i / 12)}-${pad((i % 12) + 1)}-01`, passengers: v })),
  referenceLines,
} as unknown as Rows)

describe("a tile with a forecast line", () => {
  it("hands the chart the forecast and the band, and names the model under it", () => {
    render(<TileBody spec={SPEC} cell={cellOf(36, [{ kind: "forecast", horizon: 6 }])} dark={false} loading={false} />)
    const option = drawn[drawn.length - 1].option
    expect(option.series?.map((s) => s.name)).toEqual([undefined, "Forecast band (lower)", "Forecast band", "Forecast"])
    expect(option.xAxis?.data.length).toBe(42)
    expect(screen.getByText(/^Forecast: level.*95 % band\./)).toBeTruthy()
  })

  it("says why there is no forecast and draws no line (Q5)", () => {
    render(<TileBody spec={SPEC} cell={cellOf(3, [{ kind: "forecast", horizon: 6 }])} dark={false} loading={false} />)
    expect(drawn[drawn.length - 1].option.series?.length).toBe(1)
    expect(screen.getByText("A forecast needs at least 5 periods; this chart has 3.")).toBeTruthy()
  })

  it("leaves a tile with no forecast line as it was", () => {
    render(<TileBody spec={SPEC} cell={cellOf(36, [])} dark={false} loading={false} />)
    expect(drawn[drawn.length - 1].option.series?.length).toBe(1)
    expect(screen.queryByText(/Forecast/)).toBeNull()
  })
})
