import assert from "node:assert/strict"
import test from "node:test"

import { draftFromForm, emptyMetricForm, formFromMetric, formProblems, usageSentence } from "./metric-form"

test("a metric's filters split into plain filters and segment names and join back with segments first", () => {
  const form = formFromMetric({
    label: "Total", description: "", formula: "Sum([v])",
    filters: [{ column: "g", values: ["a"] }, { segment: "domestic" }], timeColumn: "d", grain: "month",
  })
  assert.deepEqual(form.segments, ["domestic"])
  assert.equal(form.filters.length, 1)
  const draft = draftFromForm(form)
  assert.deepEqual(draft.filters, [{ segment: "domestic" }, { column: "g", values: ["a"] }])
  assert.equal(draft.grain, "month")
})

test("a grain without a date column is not sent", () => {
  const draft = draftFromForm({ ...emptyMetricForm(), label: "x", formula: "Count()", grain: "month" })
  assert.equal(draft.grain, undefined)
  assert.equal(draft.timeColumn, undefined)
})

test("save waits for a name, a label and a formula, and a description within its limit", () => {
  assert.deepEqual(formProblems(emptyMetricForm(), true, ""), ["Name", "Label", "Formula"])
  assert.deepEqual(formProblems({ ...emptyMetricForm(), label: "L", formula: "Count()" }, true, "total_visitors"), [])
  assert.deepEqual(formProblems({ ...emptyMetricForm(), label: "L", formula: "Count()" }, false, ""), [])
  assert.deepEqual(formProblems({ ...emptyMetricForm(), label: "L", formula: "Count()", description: "x".repeat(501) }, false, ""), ["Description (at most 500 characters)"])
  assert.deepEqual(formProblems({ ...emptyMetricForm(), label: "L", formula: "Count()" }, true, "1bad"), ["Name"])
})

test("the usage sentence names the charts and dashboards a change reaches, and is empty for an unused metric", () => {
  assert.equal(usageSentence({ charts: [], alerts: [], dashboards: [] }), "")
  const s = usageSentence({
    charts: [{ id: "1", title: "Visitors", kind: "bar", board: "default", boardName: "Main" }, { id: "2", title: "KPI", kind: "kpi", board: "b", boardName: "Map demo" }],
    alerts: [], dashboards: ["Main", "Map demo"],
  })
  assert.match(s, /2 charts on 2 dashboards/)
  assert.match(s, /"Visitors" on Main/)
})
