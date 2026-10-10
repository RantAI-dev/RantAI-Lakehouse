import { strict as assert } from "node:assert"
import { test } from "node:test"
import { canDrill, drillTarget, offersTileRecords, type ClickSpec } from "./chart-click"
import type { ChartKind } from "./dashboard-specs"

const spec = (kind: ChartKind, extra: Partial<ClickSpec> = {}): ClickSpec => ({
  kind, source: "ui", dimension: "region", y: "visitors", ...extra,
})

test("a category chart drills into its dimension with the clicked name", () => {
  for (const kind of ["bar", "hbar", "pie", "rose", "funnel", "treemap", "heatmap", "stacked"] as ChartKind[]) {
    assert.deepEqual(drillTarget(spec(kind), { name: "Bali" }), { column: "region", value: "Bali" }, kind)
  }
})

test("a click with no name drills nothing on a category chart", () => {
  assert.equal(drillTarget(spec("bar"), { name: "" }), null)
})

test("a chart without a category column drills nothing", () => {
  assert.equal(drillTarget(spec("bar", { dimension: "" }), { name: "Bali" }), null)
  assert.equal(drillTarget(spec("bar", { dimension: undefined }), { name: "Bali" }), null)
})

test("a built-in line or area has no value to drill, a stored one does", () => {
  for (const kind of ["line", "area"] as ChartKind[]) {
    assert.equal(canDrill(spec(kind, { source: "builtin" })), false, kind)
    assert.equal(drillTarget(spec(kind, { source: "builtin" }), { name: "2024" }), null, kind)
    assert.deepEqual(drillTarget(spec(kind), { name: "2024-01-01" }), { column: "region", value: "2024-01-01" }, kind)
  }
  assert.equal(canDrill(spec("bar", { source: "builtin" })), true)
})

test("a choropleth drills into the stored spelling, not the map's", () => {
  const regionNames = new Map([["Bandung", ["Kab. Bandung"]]])
  assert.deepEqual(
    drillTarget(spec("geomap"), { name: "Bandung", seriesType: "map" }, { regionNames }),
    { column: "region", value: "Kab. Bandung" },
  )
})

test("a choropleth region reached by two spellings, or by none, drills nothing", () => {
  const regionNames = new Map([["Bandung", ["Kab. Bandung", "BANDUNG"]]])
  assert.equal(drillTarget(spec("geomap"), { name: "Bandung" }, { regionNames }), null)
  assert.equal(drillTarget(spec("geomap"), { name: "Garut" }, { regionNames }), null)
  assert.equal(drillTarget(spec("geomap"), { name: "Bandung" }), null)
})

test("a point map drills into its label column, and nothing without a label", () => {
  assert.deepEqual(drillTarget(spec("pointmap"), { name: "Ambon #1", value: [128, -3, 5] }), { column: "region", value: "Ambon #1" })
  assert.equal(drillTarget(spec("pointmap"), { name: "", value: [128, -3, 5] }), null)
  assert.equal(drillTarget(spec("pointmap", { dimension: "" }), { name: "x" }), null)
})

test("a density map has no value to drill", () => {
  assert.equal(canDrill(spec("geoheat")), false)
  assert.equal(drillTarget(spec("geoheat"), { name: "x" }), null)
})

test("a calendar day drills into the date column with the day", () => {
  assert.deepEqual(
    drillTarget(spec("calendar", { dimension: "visit_date" }), { name: "", value: ["2026-01-03", 8] }),
    { column: "visit_date", value: "2026-01-03" },
  )
  assert.equal(drillTarget(spec("calendar"), { name: "", value: [20260103, 8] }), null)
  assert.equal(drillTarget(spec("calendar"), { name: "" }), null)
})

test("a sankey node drills into its own column and an edge into none", () => {
  const sankey = spec("sankey", { breakdown: "channel" })
  assert.deepEqual(drillTarget(sankey, { name: "a:Bali", dataType: "node" }), { column: "region", value: "Bali" })
  assert.deepEqual(drillTarget(sankey, { name: "b:Web", dataType: "node" }), { column: "channel", value: "Web" })
  assert.equal(drillTarget(sankey, { name: "a:Bali > b:Web", dataType: "edge" }), null)
  assert.equal(drillTarget(sankey, { name: "plain", dataType: "node" }), null)
})

test("a sankey value that itself starts with a side prefix keeps the rest intact", () => {
  assert.deepEqual(
    drillTarget(spec("sankey", { breakdown: "channel" }), { name: "a:a:odd", dataType: "node" }),
    { column: "region", value: "a:odd" },
  )
})

test("a sunburst ring drills into the column of that ring", () => {
  const sun = spec("sunburst", { breakdown: "channel" })
  const ring1 = { name: "Bali", treePathInfo: [{ name: "" }, { name: "Bali" }] }
  const ring2 = { name: "Web", treePathInfo: [{ name: "" }, { name: "Bali" }, { name: "Web" }] }
  assert.deepEqual(drillTarget(sun, ring1), { column: "region", value: "Bali" })
  assert.deepEqual(drillTarget(sun, ring2), { column: "channel", value: "Web" })
  assert.equal(drillTarget(sun, { name: "", treePathInfo: [{ name: "" }] }), null)
  assert.equal(drillTarget(sun, { name: "x" }), null)
})

test("a box drills into its category, not the label with its row count", () => {
  const rows = [
    { region: "Bali", visitors: [1, 2, 3, 4, 5], __n: 3 },
    { region: "Aceh", visitors: [1, 1, 1, 1, 1], __n: 1 },
  ]
  const box = spec("boxplot")
  assert.deepEqual(drillTarget(box, { name: "Aceh (n=1)", seriesType: "boxplot", dataIndex: 1 }, { rows }), { column: "region", value: "Aceh" })
  // The dot over a flat box carries the category index as its value.
  assert.deepEqual(drillTarget(box, { name: "Aceh (n=1)", seriesType: "scatter", dataIndex: 0, value: [1, 1] }, { rows }), { column: "region", value: "Aceh" })
  assert.equal(drillTarget(box, { name: "?", seriesType: "boxplot", dataIndex: 9 }, { rows }), null)
  assert.equal(drillTarget(box, { name: "?", seriesType: "boxplot", dataIndex: 0 }), null)
})

test("a box skips a row that is not a five-number summary, as the chart does", () => {
  const rows = [
    { region: "Bad", visitors: "n/a" },
    { region: "Bali", visitors: [1, 2, 3, 4, 5] },
  ]
  assert.deepEqual(drillTarget(spec("boxplot"), { name: "Bali", seriesType: "boxplot", dataIndex: 0 }, { rows }), { column: "region", value: "Bali" })
})

test("a KPI, a gauge, a table and a text tile have no value but all but text list their rows", () => {
  for (const kind of ["kpi", "gauge", "table", "text"] as ChartKind[]) {
    assert.equal(canDrill(spec(kind)), false, kind)
    assert.equal(drillTarget(spec(kind), { name: "x" }), null, kind)
  }
  assert.equal(offersTileRecords("kpi"), true)
  assert.equal(offersTileRecords("gauge"), true)
  assert.equal(offersTileRecords("table"), true)
  assert.equal(offersTileRecords("geoheat"), true)
  assert.equal(offersTileRecords("text"), false)
  assert.equal(offersTileRecords("bar"), false)
})
