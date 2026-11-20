import { describe, expect, it } from "bun:test"

import { maxSymbolRadius, thumbnailOption } from "./thumbnail-option"

/** What these tests read of a picture's option. */
type Picture = {
  grid: { left: number; right: number; top: number; bottom: number; containLabel?: boolean }
  xAxis: { show: boolean; name: string }
  yAxis: { show: boolean }[]
  legend: unknown
  calendar: Record<string, unknown>
  visualMap: Record<string, unknown>
  radar: Record<string, unknown>
}

// Owner QA 2026-11-14, gallery pictures centred.
describe("thumbnailOption", () => {
  it("gives a cartesian chart equal margins on every side, with its axes and legend off", () => {
    const o = thumbnailOption({
      grid: { left: 64, right: 16, top: 40, bottom: 28, containLabel: true },
      xAxis: { type: "category", name: "place" }, yAxis: [{ type: "value", name: "visits" }, { type: "value" }],
      legend: { top: 0 }, series: [{ type: "bar", data: [1, 2] }],
    }) as unknown as Picture
    expect(o.grid).toMatchObject({ left: 8, right: 8, top: 8, bottom: 8, containLabel: false })
    expect(o.xAxis).toMatchObject({ show: false, name: "" })
    expect(o.yAxis.map((a) => a.show)).toEqual([false, false])
    expect(o.legend).toEqual({ show: false })
  })

  it("adds a grid with equal margins to a cartesian chart that had none", () => {
    const o = thumbnailOption({ xAxis: {}, yAxis: {}, series: [] }) as unknown as Picture
    expect(o.grid).toMatchObject({ left: 8, right: 8, top: 8, bottom: 8, containLabel: false })
  })

  it("keeps room for the largest bubble, so none is clipped", () => {
    const size = (v: number[]) => 8 + 38 * Math.sqrt((v[2] ?? 0) / 28)
    const option = { xAxis: {}, yAxis: {}, series: [{ type: "scatter", symbolSize: size, data: [{ value: [1, 2, 8] }, { value: [3, 4, 28] }] }] }
    expect(maxSymbolRadius(option)).toBe(23)
    const o = thumbnailOption(option) as unknown as Picture
    expect(o.grid.left).toBeGreaterThanOrEqual(23)
    expect(o.grid.left).toBe(o.grid.right)
  })

  it("centres a calendar and hides a colour bar, keeping their colours", () => {
    const o = thumbnailOption({
      calendar: { range: ["2026-01-05", "2026-02-15"], left: 36, right: 12, top: 24, bottom: 44 },
      visualMap: { min: 0, max: 9, left: "center", bottom: 0, inRange: { color: ["#eee", "#00f"] } },
      series: [{ type: "heatmap", coordinateSystem: "calendar", data: [] }],
    }) as unknown as Picture
    expect(o.calendar).toMatchObject({ left: 8, right: 8, top: 8, bottom: 8, dayLabel: { show: false }, monthLabel: { show: false } })
    expect(o.visualMap).toMatchObject({ show: false, inRange: { color: ["#eee", "#00f"] } })
  })

  it("hides a radar's axis names and centres it", () => {
    const o = thumbnailOption({ radar: { indicator: [{ name: "Bali" }], center: ["50%", "55%"] }, series: [{ type: "radar", data: [] }] }) as unknown as Picture
    expect(o.radar).toMatchObject({ center: ["50%", "50%"], axisName: { show: false } })
  })
})
