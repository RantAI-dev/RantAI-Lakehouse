import assert from "node:assert/strict"
import test from "node:test"

import { buildCombinedOption, categoryKeys, failedLayers, type LayerCell } from "./combined-option"

type Series = { name: string; type: string; yAxisIndex: number; data: (number | null)[]; tooltip: { valueFormatter: (v: unknown) => string } }
type Axis = { position: string; axisLabel: { formatter: (v: number) => string } }

const base = [
  { m: "2026-01-01", visitors: 1500 },
  { m: "2026-02-01", visitors: 2500 },
  { m: "2026-03-01", visitors: 3500 },
]
const rooms: LayerCell = {
  label: "Occupancy", draw: "line", axis: "right", source: "mart_rooms", dimension: "month",
  columns: ["month", "rate"], rows: [{ month: "2026-02-01", rate: 0.25 }, { month: "2026-04-01", rate: 0.5 }],
}
const input = { kind: "bar", x: "m", y: "visitors", rows: base, layers: [rooms], grain: null }

test("the categories are the base's, then those only a question has, in date order", () => {
  assert.deepEqual(categoryKeys(base, "m", [rooms]), ["2026-01-01", "2026-02-01", "2026-03-01", "2026-04-01"])
})

test("text categories keep the base's order and append the new ones", () => {
  const rows = [{ p: "b", v: 1 }, { p: "a", v: 2 }]
  const layer: LayerCell = { ...rooms, dimension: "p", columns: ["p", "w"], rows: [{ p: "c", w: 1 }, { p: "a", w: 2 }] }
  assert.deepEqual(categoryKeys(rows, "p", [layer]), ["b", "a", "c"])
})

test("a value a question lacks is a gap, never a zero", () => {
  const { option, keys } = buildCombinedOption(input, false)
  const [own, layer] = option.series as Series[]
  assert.equal(keys.length, 4)
  assert.deepEqual(own.data, [1500, 2500, 3500, null])
  assert.deepEqual(layer.data, [null, 0.25, null, 0.5])
})

test("a right axis exists only when a series uses it, and each axis formats from its own series", () => {
  const both = buildCombinedOption(input, false).option
  const axes = both.yAxis as Axis[]
  assert.deepEqual(axes.map((a) => a.position), ["left", "right"])
  assert.equal(axes[0].axisLabel.formatter(2500), "2.5K")
  // The rate has decimals; the visitors do not, and neither axis borrows the other's format.
  assert.equal(axes[1].axisLabel.formatter(0.25), "0.25")
  const [own, layer] = both.series as Series[]
  assert.equal(own.tooltip.valueFormatter(1500), "1,500")
  assert.equal(layer.tooltip.valueFormatter(0.25), "0.25")
  assert.equal(layer.tooltip.valueFormatter(null), "–")
  const leftOnly = buildCombinedOption({ ...input, layers: [{ ...rooms, axis: "left" }] }, false).option
  assert.equal((leftOnly.yAxis as Axis[]).length, 1)
  assert.deepEqual((leftOnly.series as Series[]).map((s) => s.yAxisIndex), [0, 0])
})

test("each question is drawn as it asked and the chart's own series by its kind or its setting", () => {
  const [own, layer] = buildCombinedOption(input, false).option.series as Series[]
  assert.deepEqual([own.type, layer.type, own.yAxisIndex, layer.yAxisIndex], ["bar", "line", 0, 1])
  const line = buildCombinedOption({ ...input, kind: "line" }, false).option.series as Series[]
  assert.equal(line[0].type, "line")
  const forced = buildCombinedOption({ ...input, layers: [{ ...rooms, axis: "left" }], baseDraw: "line", baseAxis: "right" }, false).option.series as Series[]
  assert.deepEqual([forced[0].type, forced[0].yAxisIndex], ["line", 1])
  // Every series on the right: that is the only axis, drawn on the right.
  const rightOnly = buildCombinedOption({ ...input, baseAxis: "right" }, false).option
  assert.deepEqual((rightOnly.yAxis as Axis[]).map((a) => a.position), ["right"])
  const combo = buildCombinedOption({ ...input, kind: "combo", y: ["visitors", "visitors"] }, false)
  assert.deepEqual((combo.option.series as Series[]).slice(0, 2).map((s) => [s.type, s.yAxisIndex]), [["bar", 0], ["line", 1]])
})

test("the owner of each series is told, so a click can name its question", () => {
  const second: LayerCell = { ...rooms, label: "Second", columns: ["month", "a", "b"], rows: [{ month: "2026-01-01", a: 1, b: 2 }] }
  const built = buildCombinedOption({ ...input, layers: [rooms, second] }, false)
  assert.deepEqual(built.owners, [-1, 0, 1, 1])
  assert.deepEqual((built.option.series as Series[]).map((s) => s.name), ["visitors", "Occupancy", "Second · a", "Second · b"])
})

test("a question that failed draws nothing and is listed with its reason", () => {
  const failed: LayerCell = { ...rooms, label: "Rooms", rows: [], error: "no access", errorId: "ab12" }
  const built = buildCombinedOption({ ...input, layers: [failed, rooms] }, false)
  assert.deepEqual(built.owners, [-1, 1])
  assert.deepEqual(failedLayers([failed, rooms]), [{ label: "Rooms", error: "no access", errorId: "ab12" }])
  assert.deepEqual(failedLayers(undefined), [])
})

test("a grain labels the buckets while the keys stay the stored values", () => {
  const built = buildCombinedOption({ ...input, grain: "month" }, false)
  assert.deepEqual(built.keys.slice(0, 2), ["2026-01-01", "2026-02-01"])
  assert.deepEqual(built.labels.slice(0, 2), ["Jan 2026", "Feb 2026"])
})
