import { strict as assert } from "node:assert"
import { test } from "node:test"
import { suggestChartTitle } from "./chart-title"

test("no measure or a text note gives no suggestion", () => {
  assert.equal(suggestChartTitle({ kind: "bar", dimension: "region" }), "")
  assert.equal(suggestChartTitle({ kind: "text", measures: ["x"] }), "")
})

test("a single-value chart is named after its measure", () => {
  assert.equal(suggestChartTitle({ kind: "kpi", measures: ["visits"] }), "visits")
})

test("a category chart names the measure and the dimension", () => {
  assert.equal(suggestChartTitle({ kind: "hbar", dimension: "kelompok", measures: ["materials"] }), "materials by kelompok")
  assert.equal(
    suggestChartTitle({ kind: "sunburst", dimension: "kelompok", measures: ["materials"], breakdown: "nama" }),
    "materials by kelompok and nama",
  )
})

test("sankey, calendar, scatter and combo read the way the chart is drawn", () => {
  assert.equal(
    suggestChartTitle({ kind: "sankey", dimension: "kelompok", measures: ["materials"], breakdown: "nama" }),
    "materials: kelompok → nama",
  )
  assert.equal(suggestChartTitle({ kind: "calendar", dimension: "day", measures: ["orders"] }), "orders per day")
  assert.equal(suggestChartTitle({ kind: "scatter", dimension: "city", measures: ["price", "area"] }), "price vs area by city")
  assert.equal(suggestChartTitle({ kind: "combo", dimension: "month", measures: ["sales", "margin"] }), "sales and margin by month")
})
