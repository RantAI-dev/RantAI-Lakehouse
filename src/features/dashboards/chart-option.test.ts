import assert from "node:assert/strict"
import test from "node:test"

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
