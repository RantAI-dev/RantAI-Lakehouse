import { describe, expect, it } from "bun:test"
import type { EChartsOption } from "echarts"
import { buildOption } from "./chart-option"
import { LABEL_MAX_WIDTH, LABEL_ROTATION, categoryAxisLabel, fitCategoryLabels, fitSignature } from "./category-labels"
import { buildGeoOption } from "./geo-option"

// BI-73 a, b, c, f.
const regions = ["North Sumatra", "West Java", "East Java", "Bali", "Papua", "Aceh"]
const rows = regions.map((r, i) => ({ x: r, v: (i + 1) * 10 }))
type Axis = { axisLabel: Record<string, unknown>; data: string[] }

describe("fitCategoryLabels (BI-73 a, b)", () => {
  it("builds every vertical category axis unrotated and without a label cut", () => {
    const x = buildOption({ kind: "bar", x: "x", y: "v" }, rows, false).xAxis as Axis
    expect(x.axisLabel.rotate).toBe(0)
    expect(x.axisLabel.interval).toBe(0)
    expect(x.axisLabel.width).toBeUndefined()
  })

  it("rotates six categories on a narrow tile and keeps every label", () => {
    const option = buildOption({ kind: "bar", x: "x", y: "v" }, rows, false)
    const fitted = fitCategoryLabels(option, 328)
    const x = fitted.xAxis as Axis
    expect(x.axisLabel.rotate).toBe(LABEL_ROTATION)
    expect(x.data).toHaveLength(6)
    // Not clipped at the left edge: the grid contains the labels and keeps room for the rotated first one.
    const grid = fitted.grid as { containLabel: boolean; left: number }
    expect(grid.containLabel).toBe(true)
    expect(grid.left).toBeGreaterThan(8)
  })

  it("leaves a wide tile as it was", () => {
    const option = buildOption({ kind: "bar", x: "x", y: "v" }, rows, false)
    expect(fitCategoryLabels(option, 1200)).toBe(option)
    expect(fitCategoryLabels(option, 0)).toBe(option)
  })

  it("cuts a long label with an ellipsis at a fixed width", () => {
    const long = [{ x: "A very long category name indeed", v: 1 }, { x: "Another very long category name", v: 2 }, { x: "Short", v: 3 }]
    const fitted = fitCategoryLabels(buildOption({ kind: "bar", x: "x", y: "v" }, long, false), 300)
    const label = (fitted.xAxis as Axis).axisLabel
    expect(label.width).toBe(LABEL_MAX_WIDTH)
    expect(label.overflow).toBe("truncate")
    expect(label.hideOverlap).toBe(true)
  })

  it("covers the box plot and the combo, not a horizontal bar", () => {
    const box = buildOption({ kind: "boxplot", x: "x", y: "v" }, regions.map((r) => ({ x: r, v: [1, 2, 3, 4, 5], __n: 4 })), false)
    expect(((fitCategoryLabels(box, 328)).xAxis as Axis).axisLabel.rotate).toBe(LABEL_ROTATION)
    const combo = buildOption({ kind: "combo", x: "x", y: ["v", "v"] }, rows, false)
    expect(((fitCategoryLabels(combo, 328)).xAxis as Axis).axisLabel.rotate).toBe(LABEL_ROTATION)
    const hbar = buildOption({ kind: "hbar", x: "x", y: "v" }, rows, false)
    expect(fitCategoryLabels(hbar, 328)).toBe(hbar)
  })

  it("respects a rotation the author chose and a monthly axis", () => {
    const chosen = { xAxis: { type: "category", data: regions, axisLabel: { ...categoryAxisLabel("#000"), rotate: 45 } } } as EChartsOption
    expect(fitCategoryLabels(chosen, 200)).toBe(chosen)
    expect(fitSignature(chosen)).toBe("45||")
  })
})

describe("smooth only an ordered axis (BI-73 c)", () => {
  const smooth = (kind: "line" | "combo", cats: string[]) => {
    const option = buildOption({ kind, x: "x", y: kind === "combo" ? ["v", "v"] : "v" }, cats.map((c, i) => ({ x: c, v: i })), false)
    return (option.series as { type: string; smooth?: boolean }[]).find((s) => s.type === "line")?.smooth
  }
  it("draws straight segments over regions and curves over years", () => {
    expect(smooth("line", regions)).toBe(false)
    expect(smooth("combo", regions)).toBe(false)
    expect(smooth("line", ["2023", "2024", "2025"])).toBe(true)
    expect(smooth("combo", ["2023", "2024", "2025"])).toBe(true)
  })
  it("applies to a line with a breakdown too", () => {
    const grouped = (cats: string[]) => {
      const rs = cats.flatMap((c, i) => [{ x: c, g: "a", v: i }, { x: c, g: "b", v: i }])
      const option = buildOption({ kind: "line", x: "x", y: "v", series: "g" }, rs, false)
      return (option.series as { smooth?: boolean }[])[0].smooth
    }
    expect(grouped(regions)).toBe(false)
    expect(grouped(["2023", "2024"])).toBe(true)
  })
})

describe("the density map has a tooltip (BI-73 f)", () => {
  it("shows the place and the value", () => {
    const { option } = buildGeoOption({ kind: "geoheat", x: "name", y: "v", lat: "lat", lon: "lon", map: undefined, sqlSource: undefined } as never,
      [{ name: "Jakarta", lat: -6.2, lon: 106.8, v: 8330172 }], false)
    const tip = option.tooltip as { show: boolean; formatter: (p: unknown) => string }
    expect(tip.show).toBe(true)
    expect(tip.formatter({ name: "Jakarta", value: [106.8, -6.2, 8330172] })).toContain("Jakarta")
    expect(tip.formatter({ name: "", value: [106.8, -6.2, 1] })).toContain("-6.2000, 106.8000")
  })
})
