import assert from "node:assert/strict"
import test from "node:test"

import {
  MAX_LAYERS, dimensionsLineUp, emptyLayer, layerDraftFromDef, layerProblems, layersPayload, matchingDimensions, takesLayers, type LayerDef,
} from "./chart-layers"

const saved: LayerDef = {
  mart: "mart_rooms", dimension: "month", measures: ["rooms", { column: "hotels", aggregate: "avg" }], aggregate: "sum",
  filters: [{ column: "kind", values: ["hotel"] }], label: "Rooms", draw: "bar", axis: "left",
}

test("a bar, line, area or combo chart takes questions and no other kind does", () => {
  for (const kind of ["bar", "line", "area", "combo"]) assert.equal(takesLayers(kind), true, kind)
  for (const kind of ["hbar", "pie", "table", "kpi", "stacked", "geomap"]) assert.equal(takesLayers(kind), false, kind)
  assert.equal(MAX_LAYERS, 4)
})

test("a saved layer comes back as the same payload", () => {
  const draft = layerDraftFromDef(saved)
  assert.equal(draft.source, "mart:mart_rooms")
  assert.deepEqual(layersPayload([draft]), [{
    mart: "mart_rooms", dimension: "month", measures: ["rooms", { column: "hotels", aggregate: "avg" }], aggregate: "sum",
    filters: [{ column: "kind", values: ["hotel"] }], label: "Rooms", draw: "bar", axis: "left",
  }])
})

test("a layer over a SQL source sends sqlSource and never mart", () => {
  const draft = { ...emptyLayer(), source: "sql:s_1", dimension: "d", measures: [{ column: "w", aggregate: "" }] }
  const [sent] = layersPayload([draft])
  assert.equal(sent.sqlSource, "s_1")
  assert.equal("mart" in sent, false)
})

test("a layer's joins are kept when it is edited, not dropped", () => {
  const join = { mart: "m2", type: "left", on: [{ left: "a", op: "=", right: "b" }] }
  const draft = layerDraftFromDef({ ...saved, joins: [join] })
  assert.deepEqual(layersPayload([draft])[0].joins, [join])
})

test("a new question is a line on the right axis and is not complete", () => {
  const fresh = emptyLayer()
  assert.equal(fresh.draw, "line")
  assert.equal(fresh.axis, "right")
  assert.deepEqual(layerProblems([fresh]), ["Question 1 source", "Question 1 dimension", "Question 1 measure"])
  assert.deepEqual(layerProblems([layerDraftFromDef(saved)]), [])
  assert.deepEqual(layerProblems([{ ...layerDraftFromDef(saved), label: "x".repeat(61) }]), ["Question 1 label (at most 60 characters)"])
})

test("a dimension lines up with the chart's only when both are dates, both text or both numbers", () => {
  assert.equal(dimensionsLineUp("text", "text", false), true)
  assert.equal(dimensionsLineUp("number", "number", false), true)
  assert.equal(dimensionsLineUp("text", "number", false), false)
  assert.equal(dimensionsLineUp("date", "text", true), false)
  assert.equal(dimensionsLineUp("date", "date", false), true)
  // A date and a timestamp meet only through a time grain.
  assert.equal(dimensionsLineUp("date", "datetime", false), false)
  assert.equal(dimensionsLineUp("date", "datetime", true), true)
  // Unknown kinds are left to the server.
  assert.equal(dimensionsLineUp(undefined, "text", false), true)
})

test("only the columns that line up are offered as the question's dimension", () => {
  const columns = [{ name: "month", kind: "date" }, { name: "channel", kind: "text" }, { name: "at", kind: "datetime" }, { name: "n", kind: "number" }]
  assert.deepEqual(matchingDimensions("date", false, columns).map((c) => c.name), ["month"])
  assert.deepEqual(matchingDimensions("date", true, columns).map((c) => c.name), ["month", "at"])
  assert.deepEqual(matchingDimensions("text", false, columns).map((c) => c.name), ["channel"])
})
