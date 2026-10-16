import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ChartClickHandler } from "@/lib/chart-click"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { Rows } from "./tile-dialogs"

// The chart itself needs a canvas; the tile around it (what it hands the chart, what it lists under it) does not.
const drawn: { option: Record<string, unknown>; onDataClick?: (hit: unknown, pos: { x: number; y: number }) => void }[] = []
mock.module("./echart", () => ({
  EChart: (props: { option: Record<string, unknown>; onDataClick?: (hit: unknown, pos: { x: number; y: number }) => void }) => {
    drawn.push(props)
    return <div data-testid="chart" />
  },
}))
const { TileBody } = await import("./tile-body")

afterEach(() => { cleanup(); drawn.length = 0 })

const SPEC = {
  id: "c_1", title: "Visitors and rooms", kind: "bar", mart: "mart_visits", x: "month", y: "visitors", source: "ui",
  def: {
    dimension: "month", measures: ["visitors"],
    layers: [
      { mart: "mart_rooms", dimension: "month", measures: ["rooms"], draw: "line", axis: "right" },
      { sqlSource: "s_1", dimension: "month", measures: ["rate"], draw: "line", axis: "right" },
    ],
  },
} as unknown as ChartRenderSpec

const CELL: Rows = {
  columns: ["month", "visitors"],
  rows: [{ month: "2026-01-01", visitors: 5 }, { month: "2026-02-01", visitors: 7 }],
  layers: [
    { label: "Rooms", draw: "line", axis: "right", source: "mart_rooms", dimension: "month", columns: ["month", "rooms"], rows: [{ month: "2026-02-01", rooms: 9 }] },
    { label: "Occupancy", draw: "line", axis: "right", source: "s_1", dimension: "month", columns: [], rows: [], error: "This chart could not be loaded.", errorId: "ab12cd" },
  ],
}

describe("a tile with more questions on its chart", () => {
  it("draws every answered question on one chart and lists the one that failed, with its reason and reference", () => {
    render(<TileBody spec={SPEC} cell={CELL} dark={false} loading={false} />)
    expect(drawn).toHaveLength(1)
    const series = drawn[0].option.series as { name: string; data: (number | null)[] }[]
    expect(series.map((s) => s.name)).toEqual(["visitors", "Rooms"])
    expect(series[1].data).toEqual([null, 9])
    expect(screen.getByText("Occupancy:")).toBeTruthy()
    expect(screen.getByText("This chart could not be loaded.")).toBeTruthy()
    expect(screen.getByText("ab12cd")).toBeTruthy()
  })

  it("says under the chart which question was cut off and nothing for one that was not (BI-13 review fix R1)", () => {
    const cutCell: Rows = { ...CELL, layers: [{ ...CELL.layers![0], truncated: true }, { ...CELL.layers![0], label: "Beds" }] }
    render(<TileBody spec={SPEC} cell={cutCell} dark={false} loading={false} />)
    expect(screen.getByText("Rooms: not every value is shown")).toBeTruthy()
    expect(screen.queryByText(/Beds: not every/)).toBeNull()
  })

  it("tells a click which question the mark belongs to, with that question's own source and dimension", () => {
    const clicks: Parameters<ChartClickHandler>[] = []
    render(<TileBody spec={SPEC} cell={CELL} dark={false} loading={false} onDataClick={(...args) => { clicks.push(args) }} />)
    const click = drawn[0].onDataClick
    expect(click).toBeTruthy()
    click?.({ name: "2026-02-01", seriesIndex: 1, dataIndex: 1 }, { x: 1, y: 2 })
    click?.({ name: "2026-01-01", seriesIndex: 0, dataIndex: 0 }, { x: 1, y: 2 })
    expect(clicks[0][2].layer).toEqual({ index: 0, mart: "mart_rooms", sqlSource: undefined, dimension: "month" })
    expect(clicks[1][2].layer).toBeUndefined()
  })

  it("draws a chart with no answered question at all as no data, not as an empty axis", () => {
    const empty: Rows = { columns: ["month", "visitors"], rows: [], layers: [CELL.layers![1]] }
    render(<TileBody spec={SPEC} cell={empty} dark={false} loading={false} />)
    expect(screen.getByText("No data.")).toBeTruthy()
    expect(drawn).toHaveLength(0)
  })
})
