import assert from "node:assert/strict"
import test from "node:test"

import { tileMeasureLabels } from "@/lib/chart-shape"
import type { EChartsOption } from "echarts"
import { MARKER_LIMIT, buildOption, showsPointMarkers, withReferenceLines } from "./chart-option"

const rowsOf = (n: number) => Array.from({ length: n }, (_, i) => ({ x: `p${i}`, v: i }))
const lineSpec = { kind: "line" as const, x: "x", y: "v" }

function showSymbol(kind: "line" | "area", n: number): unknown {
  const option = buildOption({ ...lineSpec, kind }, rowsOf(n), false)
  const series = option.series as { showSymbol?: boolean }[]
  return series[0].showSymbol
}

test("a line draws point markers up to the threshold and none above it", () => {
  assert.equal(showSymbol("line", MARKER_LIMIT), true)
  assert.equal(showSymbol("line", MARKER_LIMIT + 1), false)
  assert.equal(showSymbol("line", 1000), false)
  assert.equal(MARKER_LIMIT, 60)
})

test("an area never drew point markers", () => {
  assert.equal(showSymbol("area", 5), false)
  assert.equal(showsPointMarkers("area", 5), false)
  assert.equal(showsPointMarkers("bar", 5), false)
})

test("a line with a breakdown counts its categories, not its series", () => {
  const rows = rowsOf(61).flatMap((r) => [{ ...r, g: "a" }, { ...r, g: "b" }])
  const option = buildOption({ ...lineSpec, series: "g" }, rows, false)
  const series = option.series as { showSymbol?: boolean }[]
  assert.equal(series.length, 2)
  assert.equal(series[0].showSymbol, false)
})

// BI-8 review fix (SHOULD-FIX) R4: a share between 0 and 1 is not drawn as "0".
test("a bar of shares labels its value axis and tooltip with decimals, a bar of whole numbers does not", () => {
  const axisOf = (rows: Record<string, unknown>[]) => {
    const option = buildOption({ kind: "bar", x: "c", y: "v" }, rows, false)
    const y = option.yAxis as { axisLabel: { formatter: (v: number) => string } }
    const tip = option.tooltip as { valueFormatter: (v: number) => string }
    return { tick: y.axisLabel.formatter, tip: tip.valueFormatter }
  }
  const shares = axisOf([{ c: "a", v: 0.23 }, { c: "b", v: 0.18 }, { c: "c", v: 0.32 }])
  assert.equal(shares.tick(0.1), "0.1")
  assert.equal(shares.tick(0.3), "0.3")
  assert.equal(shares.tip(0.27), "0.27")
  const whole = axisOf([{ c: "a", v: 12 }, { c: "b", v: 5 }])
  assert.equal(whole.tick(10), "10")
  assert.equal(whole.tip(1234), "1,234")
})

// BI-6: several measures on one bar or line, and the readable name of a measure with its own aggregate.
test("a bar with several measures draws one series per measure, named by its label", () => {
  const rows = [{ c: "a", visitors: 10, avg_stay: 2 }, { c: "b", visitors: 7, avg_stay: 3 }]
  const spec = { kind: "bar" as const, x: "c", y: ["visitors", "avg_stay"], measureLabels: { avg_stay: "avg of stay" } }
  const option = buildOption(spec, rows, false)
  const series = option.series as { name: string; data: number[] }[]
  assert.deepEqual(series.map((s) => s.name), ["visitors", "avg of stay"])
  assert.deepEqual(series.map((s) => s.data), [[10, 7], [2, 3]])
  assert.ok(option.legend, "several series need a legend")
})

test("a bar with one measure keeps the unnamed series it always had unless the measure has a label", () => {
  const rows = [{ c: "a", v: 10 }]
  const plain = buildOption({ kind: "bar", x: "c", y: "v" }, rows, false).series as { name?: string }[]
  assert.equal("name" in plain[0], false)
  const labelled = buildOption({ kind: "bar", x: "c", y: "avg_v", measureLabels: { avg_v: "avg of v" } }, [{ c: "a", avg_v: 4 }], false)
  assert.equal((labelled.series as { name?: string }[])[0].name, "avg of v")
})

test("a horizontal bar with several measures reverses every series with its categories", () => {
  const rows = [{ c: "a", p: 1, q: 5 }, { c: "b", p: 2, q: 6 }]
  const option = buildOption({ kind: "hbar", x: "c", y: ["p", "q"] }, rows, false)
  const series = option.series as { data: number[] }[]
  assert.deepEqual(series.map((s) => s.data), [[2, 1], [6, 5]])
  assert.deepEqual((option.yAxis as { data: string[] }).data, ["b", "a"])
})

test("a combo and a stacked bar name their series and axes by the label", () => {
  const rows = [{ c: "a", visitors: 10, avg_stay: 2 }]
  const labels = { avg_stay: "avg of stay" }
  const combo = buildOption({ kind: "combo", x: "c", y: ["visitors", "avg_stay"], measureLabels: labels }, rows, false)
  assert.deepEqual((combo.series as { name: string }[]).map((s) => s.name), ["visitors", "avg of stay"])
  assert.equal(((combo.yAxis as { name: string }[])[1]).name, "avg of stay")
  const stacked = buildOption({ kind: "stacked", x: "c", y: ["visitors", "avg_stay"], measureLabels: labels }, rows, false)
  assert.deepEqual((stacked.series as { name: string }[]).map((s) => s.name), ["visitors", "avg of stay"])
})

test("the decimals of a measure with a fraction apply to every measure on the chart", () => {
  const rows = [{ c: "a", visitors: 1200, avg_stay: 2.5 }, { c: "b", visitors: 800, avg_stay: 1.25 }]
  const option = buildOption({ kind: "bar", x: "c", y: ["visitors", "avg_stay"] }, rows, false)
  const tip = option.tooltip as { valueFormatter: (v: number) => string }
  // Largest value is over 1,000, so the compact rule keeps whole numbers; the tooltip stays consistent.
  assert.equal(tip.valueFormatter(2.5), "3")
})

test("BI-11: reference lines are dashed marks on the first series, and the value axis reaches the highest one", () => {
  const base = buildOption({ kind: "bar", x: "k", y: "v", source: "ui" } as never, [{ k: "a", v: 100 }, { k: "b", v: 200 }], false);
  assert.equal(withReferenceLines(base, [], false, false), base, "no lines: the option is returned as it is");
  assert.equal(withReferenceLines(base, undefined, false, false), base);
  const option = withReferenceLines(base, [{ value: 20000, label: "Goal" }, { value: 50 }], false, false) as unknown as {
    series: { markLine?: { data: { yAxis: number; name: string }[] } }[]; yAxis: { max: (r: { max: number }) => number };
  };
  assert.deepEqual(option.series[0].markLine?.data, [{ yAxis: 20000, name: "Goal" }, { yAxis: 50, name: "" }]);
  assert.equal(option.yAxis.max({ max: 200 }), 20000, "a target far above the data is still on the chart");
  assert.equal(option.yAxis.max({ max: 90000 }), 90000, "and the data is never cut");
});

test("BI-11: a horizontal bar draws its lines against the horizontal axis", () => {
  const base = buildOption({ kind: "hbar", x: "k", y: "v", source: "ui" } as never, [{ k: "a", v: 100 }], false);
  const option = withReferenceLines(base, [{ value: 300 }], true, true) as unknown as {
    series: { markLine?: { data: { xAxis?: number; yAxis?: number }[] } }[]; xAxis: { max: (r: { max: number }) => number };
  };
  assert.deepEqual(option.series[0].markLine?.data, [{ xAxis: 300, name: "" }]);
  assert.equal(option.xAxis.max({ max: 100 }), 300);
});

test("BI-11: a line that is not a number is not drawn", () => {
  const base = buildOption({ kind: "line", x: "k", y: "v", source: "ui" } as never, [{ k: "a", v: 1 }], false);
  assert.equal(withReferenceLines(base, [{ value: Number.NaN }], false, false), base);
});

// BI-2 review fix (SHOULD-FIX) R3: a metric reads by its label in the legend, the axis and the tooltip.
test("a chart of metrics names its series and axes by the metric's label, and by its name when it has none", () => {
  const rows = [{ c: "a", total_visitors: 10, office_visitors: 4, avg_stay: 2 }]
  const y = ["total_visitors", "office_visitors", "avg_stay"]
  const labels = tileMeasureLabels(
    [{ metric: "total_visitors" }, { metric: "office_visitors" }, { column: "stay", aggregate: "avg" }],
    y, undefined, { total_visitors: "Total visitors" },
  )
  const bar = buildOption({ kind: "bar", x: "c", y, measureLabels: labels }, rows, false)
  assert.deepEqual((bar.series as { name: string }[]).map((s) => s.name), ["Total visitors", "office_visitors", "avg of stay"])
  const combo = buildOption({ kind: "combo", x: "c", y: ["total_visitors", "office_visitors"], measureLabels: labels }, rows, false)
  assert.equal(((combo.yAxis as { name: string }[])[0]).name, "Total visitors")
  // A chart saved before the marker was kept lists the bare name; the label still follows the name.
  const bare = tileMeasureLabels(["total_visitors"], ["total_visitors"], undefined, { total_visitors: "Total visitors" })
  assert.deepEqual(bare, { total_visitors: "Total visitors" })
})

// Phase 2 QA R14, R16: a joined measure is named by its column in the legend, and a long legend entry folds with its full text in a tooltip.
test("a legend names a joined measure by its column and truncates long entries with a tooltip", () => {
  const labels = tileMeasureLabels([{ column: "mart_demo_province_capacity__rooms", aggregate: "max" }, "visitors"], ["max_mart_demo_province_capacity__rooms", "visitors"])
  assert.equal(labels.max_mart_demo_province_capacity__rooms, "max of rooms")
  const rows = [{ x: "a", max_mart_demo_province_capacity__rooms: 1, visitors: 2 }]
  const option = buildOption({ kind: "line", x: "x", y: ["max_mart_demo_province_capacity__rooms", "visitors"], measureLabels: labels }, rows, false)
  const legend = option.legend as { tooltip?: { show?: boolean }; textStyle?: { overflow?: string; width?: number } }
  assert.equal(legend.tooltip?.show, true)
  assert.equal(legend.textStyle?.overflow, "truncate")
  assert.ok((legend.textStyle?.width ?? 0) > 0)
  const names = (option.series as { name?: string }[]).map((s) => s.name)
  assert.deepEqual(names, ["max of rooms", "visitors"])
})

// PR #199 review O BLOCKER 1: ECharts sets a tooltip formatter's result as
// HTML, so a category or series name with markup ran for every viewer, on
// public links and signed embeds included. Every family of the tooltip
// formatters is fed the payload here and must write it as text.
const XSS = `<img src=x onerror=alert(1)>`
const XSS_AS_TEXT = `&lt;img src=x onerror=alert(1)&gt;`

type TooltipCase = {
  name: string
  spec: Parameters<typeof buildOption>[0]
  rows: Record<string, unknown>[]
  /** What ECharts hands the formatter. */
  params: unknown
}

const tooltipCases: TooltipCase[] = [
  { name: "pie", spec: { kind: "pie", x: "c", y: "v" }, rows: [{ c: XSS, v: 1 }], params: { name: XSS, value: 1, percent: 50 } },
  { name: "rose", spec: { kind: "rose", x: "c", y: "v" }, rows: [{ c: XSS, v: 1 }], params: { name: XSS, value: 1, percent: 50 } },
  { name: "funnel", spec: { kind: "funnel", x: "c", y: "v" }, rows: [{ c: XSS, v: 1 }], params: { name: XSS, value: 1 } },
  { name: "treemap", spec: { kind: "treemap", x: "c", y: "v" }, rows: [{ c: XSS, v: 1 }], params: { name: XSS, value: 1 } },
  { name: "scatter", spec: { kind: "scatter", x: "c", y: ["v"] }, rows: [{ c: XSS, v: 1 }], params: { name: XSS, value: [1, 2] } },
  { name: "bubble", spec: { kind: "bubble", x: "c", y: ["v", "s"] }, rows: [{ c: XSS, v: 1, s: 2 }], params: { name: XSS, value: [1, 2, 3] } },
  { name: "waterfall", spec: { kind: "waterfall", x: "c", y: "v" }, rows: [{ c: XSS, v: 1 }], params: [{ dataIndex: 0 }] },
  { name: "heatmap", spec: { kind: "heatmap", x: "c", y: "v", series: "g" }, rows: [{ c: XSS, g: XSS, v: 1 }], params: { value: [0, 0, 1] } },
  { name: "sankey", spec: { kind: "sankey", x: "c", y: "v", series: "g" }, rows: [{ c: XSS, g: XSS, v: 1 }], params: { name: `a:${XSS}`, value: 1, data: { source: `a:${XSS}`, target: `b:${XSS}` } } },
  { name: "sunburst", spec: { kind: "sunburst", x: "c", y: "v", series: "g" }, rows: [{ c: XSS, g: XSS, v: 1 }], params: { treePathInfo: [{ name: XSS }], value: 1 } },
  { name: "boxplot", spec: { kind: "boxplot", x: "c", y: "v" }, rows: [{ c: XSS, v: [1, 2, 3, 4, 5] }], params: { seriesType: "boxplot", data: [1, 2, 3, 4, 5], dataIndex: 0 } },
  { name: "calendar", spec: { kind: "calendar", x: "d", y: "v" }, rows: [{ d: "2026-10-01", v: 1 }], params: { value: [XSS, 1] } },
]

test("PR #199 review O BLOCKER 1: a name with markup in any tooltip is written as text, not as HTML", () => {
  for (const c of tooltipCases) {
    const option = buildOption(c.spec, c.rows, false) as EChartsOption
    const formatter = (option.tooltip as { formatter?: (p: unknown) => string }).formatter
    assert.equal(typeof formatter, "function", `${c.name}: the tooltip has a formatter`)
    const html = formatter!(c.params)
    assert.equal(typeof html, "string", `${c.name}: the formatter writes a string`)
    assert.ok(!html.includes("<img"), `${c.name}: the markup did not run`)
    assert.ok(html.includes(XSS_AS_TEXT), `${c.name}: the name is written as text`)
  }
})
