import assert from "node:assert/strict"
import test from "node:test"

import { builderRules, carryOverOnKindSwitch, moreOptionsUsed, type BuilderState } from "./builder-rules"

const plain: BuilderState = {
  kind: "bar", rowsMode: false, grained: false, breakdown: false, other: false, steps: 0, layers: 0,
  multiGrouped: false, limitHasEffect: true, hasSource: true,
}
const rules = (patch: Partial<BuilderState>) => builderRules({ ...plain, ...patch })

test("a text note has none of the four sections", () => {
  const r = rules({ kind: "text" })
  assert.deepEqual(r.sections, { data: false, show: false, sortAndFilter: false, moreOptions: false })
  assert.equal(r.filters, false)
  assert.equal(r.other.shown, false)
})

test("a heading, a divider and a section have no data sections, like a text note (BI-18 part C)", () => {
  for (const kind of ["heading", "divider", "section"]) {
    const r = rules({ kind })
    assert.deepEqual(r.sections, { data: false, show: false, sortAndFilter: false, moreOptions: false }, kind)
    assert.equal(r.filters, false, kind)
    assert.equal(r.parameters, false, kind)
    assert.equal(r.click, false, kind)
    assert.equal(r.sort, "none", kind)
    assert.equal(r.limit, false, kind)
    assert.equal(r.other.shown, false, kind)
  }
})

test("a bar has the whole form, with steps, questions, lines and a click offered", () => {
  const r = rules({})
  assert.deepEqual(r.sections, { data: true, show: true, sortAndFilter: true, moreOptions: true })
  assert.equal(r.sort, "choice")
  assert.equal(r.limit, true)
  assert.equal(r.other.shown, true)
  assert.equal(r.steps, true)
  assert.equal(r.layers, true)
  assert.equal(r.referenceLines, true)
  assert.equal(r.click, true)
  assert.equal(r.bins, true)
})

test("each kind gets the controls the server would accept for it", () => {
  // Steps: shape.rs `check_steps_kind`.
  for (const kind of ["bar", "hbar", "line", "area", "stacked", "combo", "pie", "rose", "funnel", "treemap", "radar", "waterfall", "table", "scatter", "bubble"]) {
    assert.equal(rules({ kind }).steps, true, `${kind} takes steps`)
  }
  for (const kind of ["kpi", "gauge", "pivot", "heatmap", "sankey", "sunburst", "calendar", "geomap", "pointmap", "geoheat", "boxplot"]) {
    assert.equal(rules({ kind }).steps, false, `${kind} takes no steps`)
  }
  // "Other": shape.rs `takes_other`.
  for (const kind of ["bar", "hbar", "stacked", "pie", "rose", "funnel", "treemap", "table"]) assert.equal(rules({ kind }).other.shown, true, kind)
  for (const kind of ["line", "area", "combo", "scatter", "kpi", "gauge", "pivot", "geomap"]) assert.equal(rules({ kind }).other.shown, false, kind)
  // Questions: layers.rs `takes_layers` (bar, line, area, combo).
  for (const kind of ["bar", "line", "area", "combo"]) assert.equal(rules({ kind }).layers, true, kind)
  for (const kind of ["hbar", "pie", "table", "kpi"]) assert.equal(rules({ kind }).layers, false, kind)
  // Lines on the value axis.
  assert.equal(rules({ kind: "pie" }).referenceLines, false)
  assert.equal(rules({ kind: "line" }).referenceLines, true)
})

test("a single number, a point map and a pivot have no sort or limit, only the filters", () => {
  for (const kind of ["kpi", "gauge", "pointmap", "geoheat", "pivot"]) {
    const r = rules({ kind })
    assert.equal(r.sort, "none", kind)
    assert.equal(r.limit, false, kind)
    assert.equal(r.filters, true, kind)
  }
  assert.equal(rules({ kind: "table", rowsMode: true }).sort, "none")
  assert.equal(rules({ kind: "table", rowsMode: true }).steps, false)
  assert.equal(rules({ kind: "boxplot" }).sort, "by-category")
  assert.equal(rules({ kind: "calendar" }).sort, "by-date")
  assert.equal(rules({ kind: "calendar" }).limit, true)
})

test("a click setting is offered only to kinds that have marks to click", () => {
  for (const kind of ["bar", "pie", "geomap", "scatter"]) assert.equal(rules({ kind }).click, true, kind)
  for (const kind of ["kpi", "gauge", "table", "geoheat"]) assert.equal(rules({ kind }).click, false, kind)
})

test("'Other' is not offered beside a time grain, several groupings or a limit with no effect", () => {
  assert.equal(rules({ grained: true }).other.shown, false)
  assert.equal(rules({ kind: "table", multiGrouped: true }).other.shown, false)
  assert.equal(rules({ limitHasEffect: false }).other.shown, false)
})

test("with 'Other' on, steps and questions are hidden and the reason is one hint at the switch", () => {
  const r = rules({ other: true })
  assert.equal(r.steps, false)
  assert.equal(r.layers, false)
  assert.equal(r.other.disabled, false)
  assert.equal(r.other.hint, "While this is on, steps and more questions are not available.")
})

test("with steps or questions present, 'Other' cannot be turned on and says why", () => {
  assert.deepEqual(rules({ steps: 1 }).other, { shown: true, disabled: true, hint: "Not available with steps." })
  assert.deepEqual(rules({ layers: 1 }).other, { shown: true, disabled: true, hint: "Not available with more questions." })
})

test("a breakdown, a time grain or questions rule steps out, and a breakdown or steps rule questions out", () => {
  assert.equal(rules({ breakdown: true }).steps, false)
  assert.equal(rules({ grained: true }).steps, false)
  assert.equal(rules({ layers: 1 }).steps, false)
  assert.equal(rules({ breakdown: true }).layers, false)
  assert.equal(rules({ steps: 1 }).layers, false)
  // Questions already on the chart keep their section so they can be removed.
  assert.equal(rules({ layers: 2, breakdown: true }).layers, true)
  assert.equal(rules({ hasSource: false }).layers, false)
})

test("questions lock the breakdown and the bins", () => {
  assert.equal(rules({ layers: 1 }).breakdownLocked, "Not available with more questions.")
  assert.equal(rules({ layers: 1 }).bins, false)
  assert.equal(rules({}).breakdownLocked, undefined)
})

test("the fold opens for anything the edited chart uses inside it", () => {
  const none = { steps: 0, layers: 0, referenceLines: 0, click: false, columnSettings: 0, compare: false }
  assert.equal(moreOptionsUsed(none), false)
  for (const key of ["steps", "layers", "referenceLines", "columnSettings"] as const) assert.equal(moreOptionsUsed({ ...none, [key]: 1 }), true, key)
  assert.equal(moreOptionsUsed({ ...none, click: true }), true)
  assert.equal(moreOptionsUsed({ ...none, compare: true }), true)
})

const sw = { rowsMode: false, dimension: "", breakdown: "", grain: "", dimensionBin: undefined, breakdownBin: undefined, groupings: [] }

test("a chart becoming a grouped table makes its dimension and breakdown the first groupings", () => {
  const patch = carryOverOnKindSwitch({ ...sw, from: "bar", to: "table", dimension: "provinsi", breakdown: "category", grain: "month" })
  assert.deepEqual(patch.groupings, [{ column: "provinsi", grain: "month" }, { column: "category" }])
  assert.equal(patch.dimension, "provinsi")
})

test("a chart without a dimension, or a table that already has groupings, is left as it is", () => {
  assert.deepEqual(carryOverOnKindSwitch({ ...sw, from: "bar", to: "table" }), {})
  assert.deepEqual(carryOverOnKindSwitch({ ...sw, from: "bar", to: "table", dimension: "a", groupings: [{ column: "b" }] }), {})
  assert.deepEqual(carryOverOnKindSwitch({ ...sw, from: "bar", to: "bar", dimension: "a" }), {})
})

test("a grouped table becoming a chart makes its first grouping the dimension", () => {
  const patch = carryOverOnKindSwitch({ ...sw, from: "table", to: "bar", groupings: [{ column: "provinsi", grain: "month" }, { column: "category" }] })
  assert.equal(patch.dimension, "provinsi")
  assert.equal(patch.grain, "month")
  assert.equal(patch.bin, null)
  // Raw rows have no groupings to carry.
  assert.deepEqual(carryOverOnKindSwitch({ ...sw, from: "table", to: "bar", rowsMode: true, groupings: [] }), {})
})
