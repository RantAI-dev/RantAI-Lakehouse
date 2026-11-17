import { cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { Rows } from "./tile-dialogs"

const drawn: { option: Record<string, unknown> }[] = []
mock.module("./echart", () => ({
  EChart: (props: { option: Record<string, unknown> }) => {
    drawn.push(props)
    return <div data-testid="chart" />
  },
}))
const { TileBody } = await import("./tile-body")

afterEach(() => { cleanup(); drawn.length = 0 })

// BI-11 review fix (BLOCKER) R1: a measure chosen by a list parameter. The stored definition keeps `{ukuran}`;
// the server resolves the slot and names the result column (`sum_rooms`) in `y`, and says what it resolved to.
const SPEC = {
  id: "u_1", title: "Capacity", kind: "bar", mart: "mart_cap", x: "provinsi", y: "sum_rooms", source: "ui",
  def: { dimension: "provinsi", measures: [{ column: "{ukuran}", aggregate: "sum" }] },
  resolved: { dimension: "provinsi", breakdown: null, measures: [{ column: "rooms", aggregate: "sum" }] },
} as unknown as ChartRenderSpec

const CELL: Rows = { columns: ["provinsi", "sum_rooms"], rows: [{ provinsi: "A", sum_rooms: 5 }, { provinsi: "B", sum_rooms: 9 }] }

describe("a tile whose measure column is chosen by a parameter", () => {
  it("draws the resolved column under the resolved name, not the parameter's text", () => {
    render(<TileBody spec={SPEC} cell={CELL} dark={false} loading={false} />)
    const series = drawn[0].option.series as { name: string; data: unknown[] }[]
    expect(series).toHaveLength(1)
    expect(series[0].data.map((d) => (typeof d === "object" && d ? (d as { value: unknown }).value : d))).toEqual([5, 9])
    expect(series[0].name).toBe("sum of rooms")
    expect(JSON.stringify(drawn[0].option)).not.toContain("{ukuran}")
  })

  it("follows the parameter when its value changes", () => {
    const hotels = { ...SPEC, y: "sum_hotels", resolved: { dimension: "provinsi", breakdown: null, measures: [{ column: "hotels", aggregate: "sum" }] } } as unknown as ChartRenderSpec
    render(<TileBody spec={hotels} cell={{ columns: ["provinsi", "sum_hotels"], rows: [{ provinsi: "A", sum_hotels: 195 }] }} dark={false} loading={false} />)
    const series = drawn[0].option.series as { name: string }[]
    expect(series[0].name).toBe("sum of hotels")
  })
})
