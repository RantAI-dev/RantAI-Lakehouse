import { describe, expect, test } from "bun:test"
import type { EChartsOption } from "echarts"
import { decimalsFor } from "@/lib/chart-axis"
import type { StyleFields } from "@/lib/chart-style"
import { buildOption, computedValue, trendEnds, withReferenceLines } from "./chart-option"
import { seriesColumns, styleOption } from "./style-option"

const rows = [
  { k: "Bali", v: 1200, w: 3 },
  { k: "Java", v: 2400, w: 5 },
  { k: "Sumatra", v: 600, w: 4 },
]

function draw(kind: string, style: StyleFields | undefined, y: string | string[] = "v", extra: Record<string, unknown> = {}, locale = "en-US") {
  const spec = { kind, x: "k", y, ...extra } as never
  const values = rows.flatMap((r) => (Array.isArray(y) ? y : [y]).map((c) => Number(r[c as "v"])))
  const option = buildOption(spec, rows, false, { style, locale, companyPalette: ["#112233", "#445566"] })
  const styled = styleOption(option, { kind, y, style, locale, decimals: decimalsFor(values), breakdown: !!extra.series })
  return styled
}
const series = (o: EChartsOption) => o.series as Record<string, unknown>[]

describe("number formats", () => {
  test("the axis, the tooltip and the labels write the chart's format in the deployment's locale", () => {
    const style: StyleFields = { format: { style: "compact", currency: "IDR", decimals: 2 }, labels: { show: true } }
    const { option } = draw("bar", style, "v", {}, "id-ID")
    const axis = option.yAxis as { axisLabel: { formatter: (v: number) => string } }
    expect(axis.axisLabel.formatter(1_234_567)).toBe("Rp 1,23 M")
    const tip = option.tooltip as { valueFormatter: (v: number) => string }
    expect(tip.valueFormatter(1_234_567.891)).toBe("Rp 1,23 M")
    const label = series(option)[0]!.label as { formatter: (p: { value: number }) => string }
    expect(label.formatter({ value: 2400 })).toBe("Rp 2,40 K")
  })

  test("a chart with no style draws exactly what it drew before", () => {
    const spec = { kind: "bar", x: "k", y: "v" } as never
    const a = buildOption(spec, rows, false)
    const b = buildOption(spec, rows, false, { style: undefined, locale: "en-US" })
    expect(JSON.stringify(a)).toBe(JSON.stringify(b))
    expect(styleOption(a, { kind: "bar", y: "v", style: undefined, locale: "en-US", decimals: 0 }).option).toBe(a)
  })

  test("a per-value format overrides the chart's for its own series", () => {
    const style: StyleFields = { format: { style: "number" }, valueFormats: { w: { style: "percent", decimals: 0 } } }
    const { option } = draw("bar", style, ["v", "w"])
    const second = series(option)[1]!.tooltip as { valueFormatter: (v: number) => string }
    expect(second.valueFormatter(0.4)).toBe("40%")
    expect(series(option)[0]!.tooltip).toBeUndefined()
  })
})

describe("colours", () => {
  test("a palette sets the series colours and the company palette is used when chosen", () => {
    expect(draw("bar", { colors: { palette: "muted" } }).option.color).toContain("#5a7daf")
    expect(draw("bar", { colors: { palette: "company" } }).option.color).toEqual(["#112233", "#445566"])
    expect(draw("bar", undefined).option.color).toContain("#6366f1")
  })

  test("a colour per series applies to the series of that name", () => {
    const named = draw("bar", { colors: { series: { "Total": "#00aa00" } } }, ["v", "w"], { measureLabels: { v: "Total", w: "Other" } }).option
    expect((series(named)[0]!.itemStyle as { color: string }).color).toBe("#00aa00")
    expect((series(named)[1]!.itemStyle as { color: string }).color).not.toBe("#00aa00")
  })

  test("colour by value ranges from the first to the last colour over the shown minimum and maximum", () => {
    const { option } = draw("bar", { colors: { byValue: ["#000000", "#ffffff"] } })
    const data = series(option)[0]!.data as { value: number; itemStyle: { color: string } }[]
    expect(data.map((d) => d.itemStyle.color)).toEqual(["#555555", "#ffffff", "#000000"])
  })
})

describe("axes", () => {
  test("title, limits, rotation and hiding apply to the right axes, also on a horizontal bar", () => {
    const style: StyleFields = { axes: { x: { title: "Province", rotate: 45 }, y: { title: "Visitors", min: 0, max: 5000, show: false } } }
    const v = draw("bar", style).option as unknown as { xAxis: Record<string, unknown>; yAxis: Record<string, unknown> }
    expect(v.xAxis.name).toBe("Province")
    expect((v.xAxis.axisLabel as { rotate: number }).rotate).toBe(45)
    expect(v.yAxis).toMatchObject({ name: "Visitors", min: 0, max: 5000, show: false })
    const h = draw("hbar", style).option as unknown as { xAxis: Record<string, unknown>; yAxis: Record<string, unknown> }
    expect(h.yAxis.name).toBe("Province")
    expect(h.xAxis).toMatchObject({ name: "Visitors", min: 0, max: 5000 })
  })

  test("a logarithmic axis is drawn when every value is above zero", () => {
    const { option, message } = draw("bar", { axes: { y: { log: true } } })
    expect(message).toBeUndefined()
    expect((option.yAxis as { type: string }).type).toBe("log")
  })

  test("a logarithmic axis with a zero value returns the message and no option", () => {
    const zero = [...rows, { k: "Papua", v: 0, w: 1 }]
    const option = buildOption({ kind: "bar", x: "k", y: "v" } as never, zero, false)
    const styled = styleOption(option, { kind: "bar", y: "v", style: { axes: { y: { log: true } } }, locale: "en-US", decimals: 0 })
    expect(styled.message).toBe("A logarithmic axis cannot show zero or negative values")
    expect(styled.option).toBe(option)
  })

  test("values on the right go to a second Y axis", () => {
    const { option } = draw("bar", { axes: { rightValues: ["w"], y2: { title: "Nights" } } }, ["v", "w"])
    const axes = option.yAxis as { name?: string; position?: string }[]
    expect(axes).toHaveLength(2)
    expect(axes[1]).toMatchObject({ position: "right", name: "Nights" })
    expect(series(option).map((s) => s.yAxisIndex)).toEqual([0, 1])
  })
})

describe("data labels and notes", () => {
  test("labels are written in the value's format and overlapping ones are dropped", () => {
    const { option } = draw("line", { labels: { show: true }, format: { style: "number", decimals: 1 } })
    const s = series(option)[0]!
    expect((s.label as { show: boolean; formatter: (p: { value: number }) => string }).formatter({ value: 3 })).toBe("3.0")
    expect(s.labelLayout).toEqual({ hideOverlap: true })
  })

  test("a note becomes a pin on its point, and a note whose point is gone is counted, not drawn", () => {
    const style: StyleFields = { annotations: [{ x: "Java", text: "Peak season" }, { x: "Papua", text: "Gone" }] }
    const { option, hiddenNotes } = draw("bar", style)
    const pin = (series(option)[0]!.markPoint as { data: { name: string; coord: number[]; value: number }[] }).data
    expect(pin).toEqual([{ name: "Peak season", coord: [1, 2400], value: 2400 }])
    expect(hiddenNotes).toBe(1)
  })

  test("the tooltip lists only the fields chosen", () => {
    const { option } = draw("bar", { tooltip: { fields: ["w"] } }, ["v", "w"])
    const format = (option.tooltip as { formatter: (p: unknown) => string }).formatter
    const html = format([
      { axisValueLabel: "Bali", seriesIndex: 0, seriesName: "v", value: 1200 },
      { axisValueLabel: "Bali", seriesIndex: 1, seriesName: "w", value: 3 },
    ])
    expect(html).toContain("w")
    expect(html).not.toContain("1,200")
  })
})

describe("series columns", () => {
  test("a breakdown draws one value in every series and a combo its two values", () => {
    expect(seriesColumns({ kind: "bar", y: "v", breakdown: true }, 3)).toEqual(["v", "v", "v"])
    expect(seriesColumns({ kind: "combo", y: ["a", "b"] }, 2)).toEqual(["a", "b"])
    expect(seriesColumns({ kind: "waterfall", y: "v" }, 3)).toEqual([undefined, "v", "v"])
  })
})

describe("reference lines", () => {
  const ctx = (cutOff = false) => ({ values: () => rows.map((r) => r.v), cutOff, format: (_m: string | undefined, v: number) => String(Math.round(v)) })
  const base = () => buildOption({ kind: "bar", x: "k", y: "v" } as never, rows, false)
  type Marks = { series: { markLine?: { data: Record<string, unknown>[] }; markArea?: { data: unknown[][] } }[] }

  test("average, median, min and max are computed from the rows on screen", () => {
    expect(computedValue("average", [1, 2, 6])).toBe(3)
    expect(computedValue("median", [5, 1, 3])).toBe(3)
    expect(computedValue("median", [1, 2, 3, 4])).toBe(2.5)
    expect(computedValue("min", [4, 2, 9])).toBe(2)
    expect(computedValue("max", [4, 2, 9])).toBe(9)
    expect(computedValue("average", [])).toBeNull()
  })

  test("a goal line at 1,000 is drawn on a bar chart", () => {
    const o = withReferenceLines(base(), [{ value: 1000, label: "Goal" }], false, false, ctx()) as unknown as Marks
    expect(o.series[0]!.markLine?.data).toEqual([{ yAxis: 1000, name: "Goal" }])
  })

  test("an average line is named, valued from the data and says 'of the N shown' when cut off", () => {
    const whole = withReferenceLines(base(), [{ kind: "average" }], false, false, ctx()) as unknown as Marks
    expect(whole.series[0]!.markLine?.data).toEqual([{ yAxis: 1400, name: "Average" }])
    const cut = withReferenceLines(base(), [{ kind: "max", label: "Peak" }], false, false, ctx(true)) as unknown as Marks
    expect(cut.series[0]!.markLine?.data).toEqual([{ yAxis: 2400, name: "Peak of the 3 shown" }])
  })

  test("a trend line is the least-squares line over the shown points", () => {
    expect(trendEnds([1, 2, 3])).toEqual([1, 3])
    expect(trendEnds([5])).toBeNull()
    const o = withReferenceLines(base(), [{ kind: "trend" }], false, false, ctx()) as unknown as Marks
    const [from, to] = o.series[0]!.markLine!.data[0] as unknown as [{ coord: number[]; name: string }, { coord: number[] }]
    expect(from.coord[0]).toBe(0)
    expect(to.coord[0]).toBe(2)
    expect(from.name).toBe("Trend")
    // 1200, 2400, 600: slope -300 through mean 1400.
    expect(Math.round(from.coord[1]!)).toBe(1700)
    expect(Math.round(to.coord[1]!)).toBe(1100)
  })

  test("a band is a shaded area between two values and widens the axis", () => {
    const o = withReferenceLines(base(), [{ kind: "band", value: 500, to: 9000, label: "Normal", color: "#00aa00" }], false, false, ctx()) as unknown as Marks & { yAxis: { max: (r: { max: number }) => number } }
    expect(o.series[0]!.markArea?.data).toHaveLength(1)
    expect(o.yAxis.max({ max: 100 })).toBe(9000)
  })

  test("a computed line with nothing on screen to compute from is left out", () => {
    const empty = { values: () => [] as number[] }
    const o = base()
    expect(withReferenceLines(o, [{ kind: "average" }], false, false, empty)).toBe(o)
  })
})
