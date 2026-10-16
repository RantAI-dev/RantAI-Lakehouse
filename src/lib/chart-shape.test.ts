import assert from "node:assert/strict"
import test from "node:test"

import {
  columnRef, metricColumn, effectiveOrder, entriesFromRows, isOtherRow, labelOtherRows, measureLabel, measureLimits, parseSortChoice,
  resultName, rowsFromEntries, shapeDraftFromDef, shapePayload, shapeProblems, sortChoiceOf, sortOptions,
  tileMeasureLabels, usesGroupings, emptyShapeDraft, effectiveAggregate, storedAggregate, type MeasureEntry,
} from "./chart-shape"

test("a plain name stays a name and an own aggregate becomes an object", () => {
  const rows = [{ column: "visitors", aggregate: "" }, { column: "stay", aggregate: "avg" }, { column: "", aggregate: "" }]
  assert.deepEqual(entriesFromRows(rows, true), ["visitors", { column: "stay", aggregate: "avg" }])
  // A kind that takes the chart's aggregate only sends names.
  assert.deepEqual(entriesFromRows(rows, false), ["visitors", "stay"])
})

test("saved measures come back as rows, and none gives one empty row", () => {
  assert.deepEqual(rowsFromEntries(["a", { column: "b", aggregate: "max" }]), [
    { column: "a", aggregate: "" }, { column: "b", aggregate: "max" },
  ])
  assert.deepEqual(rowsFromEntries(undefined), [{ column: "", aggregate: "" }])
})

test("two aggregations of one column get different result names and readable labels", () => {
  const none = new Set<string>()
  const avg = { column: "stay", aggregate: "avg" }
  const max = { column: "stay", aggregate: "max" }
  assert.equal(resultName(avg, none), "avg_stay")
  assert.equal(resultName(max, none), "max_stay")
  assert.equal(measureLabel(avg, none), "avg of stay")
  assert.equal(resultName({ column: "stay", aggregate: "" }, none), "stay")
})

test("a calculated aggregate keeps its own name whatever aggregate is chosen", () => {
  const whole = new Set(["ratio"])
  assert.equal(resultName({ column: "ratio", aggregate: "avg" }, whole), "ratio")
  assert.equal(measureLabel({ column: "ratio", aggregate: "avg" }, whole), "ratio")
})

test("the number of measures follows the kind and a fixed role takes no own aggregate beyond its count", () => {
  assert.deepEqual(measureLimits("bar"), { min: 1, max: 6, own: true })
  assert.deepEqual(measureLimits("combo"), { min: 2, max: 2, own: true })
  assert.deepEqual(measureLimits("bubble"), { min: 3, max: 3, own: true })
  assert.deepEqual(measureLimits("kpi"), { min: 1, max: 1, own: false })
  assert.deepEqual(measureLimits("table", true), { min: 0, max: 0, own: false })
})

test("the sort control offers the three orders, then each grouping and measure both ways", () => {
  const options = sortOptions(["kab"], [{ name: "avg_stay", label: "avg of stay" }])
  assert.deepEqual(options.map((o) => o.value), [
    "order:desc", "order:asc", "order:none", "by:kab:asc", "by:kab:desc", "by:avg_stay:desc", "by:avg_stay:asc",
  ])
  assert.equal(options.find((o) => o.value === "by:avg_stay:desc")?.label, "avg of stay, highest first")
})

test("a sort choice round-trips through the definition", () => {
  assert.deepEqual(parseSortChoice("order:asc"), { order: "asc" })
  assert.deepEqual(parseSortChoice("by:avg_stay:desc"), { sort: { by: "avg_stay", dir: "desc" } })
  assert.equal(sortChoiceOf({ order: "none" }), "order:none")
  assert.equal(sortChoiceOf({ sort: { by: "kab", dir: "asc" } }), "by:kab:asc")
  assert.equal(sortChoiceOf({}), "order:desc")
})

test("a sort by a grouping reads as the natural order and by a measure as a value order", () => {
  assert.equal(effectiveOrder("by:kab:desc", ["kab"]), "none")
  assert.equal(effectiveOrder("by:avg_stay:asc", ["kab"]), "asc")
  assert.equal(effectiveOrder("order:none", ["kab"]), "none")
})

test("the draft sends only what its kind and the other choices allow", () => {
  const draft = {
    ...emptyShapeDraft(),
    filters: [{ column: "category", values: ["Office"] }],
    sortChoice: "by:visitors:desc",
    other: true,
  }
  const bar = shapePayload(draft, { kind: "bar", rowsMode: false, grained: false })
  assert.deepEqual(bar, { filters: draft.filters, sort: { by: "visitors", dir: "desc" }, other: true })
  // A time grain, a line chart and a raw table take no "Other".
  assert.equal(shapePayload(draft, { kind: "bar", rowsMode: false, grained: true }).other, undefined)
  assert.equal(shapePayload(draft, { kind: "line", rowsMode: false, grained: false }).other, undefined)
  assert.equal(shapePayload(draft, { kind: "table", rowsMode: true, grained: false }).other, undefined)
  // A KPI keeps its filters but takes no sort.
  const kpi = shapePayload(draft, { kind: "kpi", rowsMode: false, grained: false })
  assert.equal(kpi.sort, undefined)
  assert.deepEqual(kpi.filters, draft.filters)
  // A text tile has no data to filter.
  assert.equal(shapePayload(draft, { kind: "text", rowsMode: false, grained: false }).filters, undefined)
})

test("a legacy order sends no sort and a table sends groupings only when it needs them", () => {
  const one = { ...emptyShapeDraft(), groupings: [{ column: "kab" }] }
  assert.equal(usesGroupings(one.groupings), false)
  assert.equal(shapePayload(one, { kind: "table", rowsMode: false, grained: false }).groupings, undefined)
  const two = { ...emptyShapeDraft(), groupings: [{ column: "kab" }, { column: "d", grain: "month" }], other: true }
  assert.equal(usesGroupings(two.groupings), true)
  const payload = shapePayload(two, { kind: "table", rowsMode: false, grained: false })
  assert.deepEqual(payload.groupings, [{ column: "kab" }, { column: "d", grain: "month" }])
  // "Other" needs one dimension.
  assert.equal(payload.other, undefined)
  assert.equal(usesGroupings([{ column: "d", grain: "month" }]), true)
})

test("a saved definition becomes a draft and a table needs a grouping", () => {
  const draft = shapeDraftFromDef({
    filters: [{ column: "c", values: ["a"] }], sort: { by: "v", dir: "desc" }, other: true, groupings: [{ column: "c" }],
  })
  assert.equal(draft.sortChoice, "by:v:desc")
  assert.equal(draft.other, true)
  assert.deepEqual(shapeProblems(emptyShapeDraft(), { kind: "table", rowsMode: false }), ["Group by"])
  assert.deepEqual(shapeProblems(draft, { kind: "table", rowsMode: false }), [])
  assert.deepEqual(shapeProblems(emptyShapeDraft(), { kind: "bar", rowsMode: false }), [])
})

test("the computed Other group is recognised by its flag, not by its label", () => {
  assert.equal(isOtherRow({ __other: 1 }), true)
  assert.equal(isOtherRow({ __other: "1" }), true)
  assert.equal(isOtherRow({ __other: 0, kab: "Other" }), false)
  assert.equal(isOtherRow({ kab: "Other" }), false)
})

test("a real value called Other is kept apart from the computed group", () => {
  const rows = [
    { kab: "Other", v: 100, __other: 0 },
    { kab: "A", v: 50, __other: 0 },
    { kab: "Other", v: 90, __other: 1 },
  ]
  const { rows: drawn, otherLabels } = labelOtherRows(rows, "kab")
  assert.deepEqual(drawn.map((r) => r.kab), ["Other", "A", "Other (rest)"])
  assert.deepEqual([...otherLabels], ["Other (rest)"])
  // Without a collision the computed group is simply "Other".
  const plain = labelOtherRows([{ kab: "A", v: 1, __other: 0 }, { kab: "x", v: 2, __other: 1 }], "kab")
  assert.deepEqual(plain.rows.map((r) => r.kab), ["A", "Other"])
  assert.deepEqual([...plain.otherLabels], ["Other"])
  assert.equal(labelOtherRows([{ kab: "A", v: 1 }], "kab").otherLabels.size, 0)
})

test("a tile labels a measure with its own aggregate by position", () => {
  const labels = tileMeasureLabels(["visitors", { column: "stay", aggregate: "avg" }], ["visitors", "avg_stay"])
  assert.deepEqual(labels, { avg_stay: "avg of stay" })
  // A calculated aggregate kept its name: no label to add.
  assert.deepEqual(tileMeasureLabels([{ column: "ratio", aggregate: "avg" }], "ratio"), {})
  assert.deepEqual(tileMeasureLabels(undefined, "v"), {})
})

// ── BI-6 part 2: steps ───────────────────────────────────────────────────

import {
  cleanStep, columnsAfterSteps, conditionComplete, isInternalColumn, stepsApply, takesSteps, visibleColumns,
  type ResultColumns, type StepDef,
} from "./chart-shape"

const base: ResultColumns = {
  groups: [{ name: "kab", kind: "text" }, { name: "d", kind: "date" }],
  measures: ["visitors", "avg_stay"],
  labels: { avg_stay: "avg of stay" },
  bases: { avg_stay: "stay" },
}
const summarise: StepDef = { summarise: { by: ["kab"], measures: [{ column: "visitors", aggregate: "sum" }, { column: "avg_stay", aggregate: "max" }] } }

test("a summary leaves the groupings it keeps and measures named by aggregate and column", () => {
  const after = columnsAfterSteps(base, [summarise])
  assert.deepEqual(after.groups, [{ name: "kab", kind: "text" }])
  assert.deepEqual(after.measures, ["sum_visitors", "max_avg_stay"])
  // Phase 2 QA R8: plain words and the original column, never "max of avg of stay".
  assert.equal(after.labels.sum_visitors, "total visitors")
  assert.equal(after.labels.max_avg_stay, "highest stay")
})

test("a second summary names its output from the original column and keeps the result name when two would read the same", () => {
  const twice = columnsAfterSteps(base, [summarise, { summarise: { by: ["kab"], measures: [{ column: "sum_visitors", aggregate: "avg" }] } }])
  assert.equal(twice.labels.avg_sum_visitors, "average visitors")
  const clash = columnsAfterSteps(
    { groups: base.groups, measures: ["visitors", "sum_visitors"], labels: {}, bases: { sum_visitors: "visitors" } },
    [{ summarise: { by: ["kab"], measures: [{ column: "visitors", aggregate: "sum" }, { column: "sum_visitors", aggregate: "sum" }] } }],
  )
  assert.equal(clash.labels.sum_visitors, "sum_visitors")
  assert.equal(clash.labels.sum_sum_visitors, "sum_sum_visitors")
})

test("a filter step leaves the columns as they were and a second summary reads the first", () => {
  const filter: StepDef = { filter: [{ column: "sum_visitors", op: "gt", value: "1000" }] }
  assert.deepEqual(columnsAfterSteps(base, [filter]), base)
  const twice = columnsAfterSteps(base, [summarise, { summarise: { by: ["kab"], measures: [{ column: "sum_visitors", aggregate: "avg" }] } }])
  assert.deepEqual(twice.measures, ["avg_sum_visitors"])
})

test("the steps are sent only where the chart takes them", () => {
  const draft = { ...emptyShapeDraft(), steps: [summarise] }
  const ctx = { kind: "bar", rowsMode: false, grained: false }
  assert.deepEqual(shapePayload(draft, ctx).steps, [summarise])
  for (const blocked of [{ ...ctx, grained: true }, { ...ctx, breakdown: true }, { ...ctx, kind: "kpi" }, { ...ctx, kind: "pivot" }, { ...ctx, kind: "table", rowsMode: true }, { ...ctx, kind: "geomap" }]) {
    assert.equal(shapePayload(draft, blocked).steps, undefined, JSON.stringify(blocked))
  }
  assert.equal(stepsApply({ ...draft, other: true }, ctx), false)
  // "Other" wins when both are set: steps are not sent beside it.
  assert.equal(shapePayload({ ...draft, other: true }, ctx).steps, undefined)
  assert.equal(shapePayload({ ...draft, other: true }, ctx).other, true)
  assert.equal(takesSteps("bar"), true)
  assert.equal(takesSteps("kpi"), false)
})

test("a step with nothing filled in is a problem to fix before the preview", () => {
  const ctx = { kind: "bar", rowsMode: false }
  const withSteps = (steps: StepDef[]) => ({ ...emptyShapeDraft(), steps })
  assert.deepEqual(shapeProblems(withSteps([{ filter: [{ column: "visitors", op: "gt", value: "" }] }]), ctx), ["Step 1 value"])
  assert.deepEqual(shapeProblems(withSteps([{ filter: [] }]), ctx), ["Step 1 condition"])
  assert.deepEqual(shapeProblems(withSteps([{ summarise: { by: [], measures: [] } }]), ctx), ["Step 1 grouping", "Step 1 measure"])
  assert.deepEqual(shapeProblems(withSteps([{ filter: [{ column: "visitors", op: "gt", value: "5" }] }]), ctx), [])
  // Steps that are not sent are not a problem.
  assert.deepEqual(shapeProblems(withSteps([{ filter: [] }]), { ...ctx, grained: true }), [])
  assert.equal(conditionComplete({ column: "v", op: "between", min: "", max: "5" }), true)
  assert.equal(conditionComplete({ column: "v", op: "between" }), false)
})

test("a step is sent trimmed and without the empty fields of the other operator", () => {
  assert.deepEqual(cleanStep({ filter: [{ column: "v", op: "gt", value: " 5 ", min: "1" }, { column: "v", op: "between", min: " 1", max: "" }] }), {
    filter: [{ column: "v", op: "gt", value: "5" }, { column: "v", op: "between", min: "1" }],
  })
})

test("the labels of a tile with steps come from the last step, not from the measures by position", () => {
  const labels = tileMeasureLabels(["visitors"], ["sum_visitors"], [{ summarise: { by: ["kab"], measures: [{ column: "visitors", aggregate: "sum" }] } }])
  assert.deepEqual(labels, { sum_visitors: "total visitors" })
  assert.deepEqual(tileMeasureLabels([{ column: "stay", aggregate: "avg" }], "avg_stay", [{ filter: [{ column: "avg_stay", op: "gt", value: "1" }] }]), { avg_stay: "avg of stay" })
  // BI-2 review fix (SHOULD-FIX) R3: the tile payload's label wins for a metric; a column or an own aggregate keeps its reading.
  assert.deepEqual(
    tileMeasureLabels([{ metric: "total_visitors" }, "visitors", { column: "stay", aggregate: "avg" }], ["total_visitors", "visitors", "avg_stay"], undefined, { total_visitors: "Total visitors" }),
    { total_visitors: "Total visitors", avg_stay: "avg of stay" },
  )
})

test("internal columns are left out of every list a person reads", () => {
  assert.equal(isInternalColumn("__other"), true)
  assert.equal(isInternalColumn("__s0"), true)
  assert.equal(isInternalColumn("_x"), false)
  assert.deepEqual(visibleColumns(["kab", "visitors", "__other", "__n"]), ["kab", "visitors"])
})

test("a joined column is labelled by its own name in a tile's legend, including an own aggregate (BI-7)", () => {
  assert.deepEqual(tileMeasureLabels(["visitors", "capacity__hotels"], ["visitors", "capacity__hotels"]), { capacity__hotels: "hotels" })
  assert.deepEqual(
    tileMeasureLabels(["visitors", { column: "capacity__rooms", aggregate: "max" }], ["visitors", "max_capacity__rooms"]),
    { max_capacity__rooms: "max of rooms" },
  )
})

test("a bin is sent for the dimension and the breakdown, never with a grain, and a grid only for a point map", () => {
  const draft = { ...emptyShapeDraft(), bin: { mode: "width" as const, count: "10", width: "500" }, breakdownBin: { mode: "auto" as const, count: "10", width: "" }, grid: "1" }
  const bar = shapePayload(draft, { kind: "bar", rowsMode: false, grained: false, breakdown: true })
  assert.deepEqual(bar.bin, { mode: "width", width: 500 })
  assert.deepEqual(bar.breakdownBin, { mode: "auto" })
  assert.equal(bar.grid, undefined)
  // Without a breakdown the breakdown's bin is not sent; with a grain, no bin at all.
  assert.equal(shapePayload(draft, { kind: "bar", rowsMode: false, grained: false }).breakdownBin, undefined)
  assert.equal(shapePayload(draft, { kind: "bar", rowsMode: false, grained: true }).bin, undefined)
  assert.equal(shapePayload(draft, { kind: "kpi", rowsMode: false, grained: false }).bin, undefined)
  assert.deepEqual(shapePayload(draft, { kind: "pointmap", rowsMode: false, grained: false }).grid, { size: 1 })
  assert.deepEqual(shapePayload({ ...draft, grid: "auto" }, { kind: "geoheat", rowsMode: false, grained: false }).grid, { size: "auto" })
})

test("a grouped table sends the bin of a grouping and one grouping with a bin is already `groupings`", () => {
  const draft = { ...emptyShapeDraft(), groupings: [{ column: "visitors", bin: { mode: "count" as const, count: "50" } }] }
  const out = shapePayload(draft, { kind: "table", rowsMode: false, grained: false })
  assert.deepEqual(out.groupings, [{ column: "visitors", bin: { mode: "count", count: 50 } }])
  assert.equal(usesGroupings(draft.groupings), true)
})

test("a bin by width without its width is listed as still needed", () => {
  const draft = { ...emptyShapeDraft(), bin: { mode: "width" as const, count: "10", width: "" } }
  assert.deepEqual(shapeProblems(draft, { kind: "bar", rowsMode: false }), ["Dimension bin width"])
  assert.deepEqual(shapeProblems({ ...draft, bin: { ...draft.bin, width: "5" } }, { kind: "bar", rowsMode: false }), [])
})

test("a saved bin and grid come back as a draft", () => {
  const draft = shapeDraftFromDef({ bin: { mode: "count", count: 50 }, breakdownBin: { mode: "auto" }, grid: { size: 0.1 } })
  assert.deepEqual(draft.bin, { mode: "count", count: "50", width: "" })
  assert.equal(draft.breakdownBin?.mode, "auto")
  assert.equal(draft.grid, "0.1")
  assert.equal(shapeDraftFromDef({}).bin, null)
})

test("a metric is a measure row named metric:<name> and is sent as { metric }, with no aggregate of its own (BI-2)", async () => {
  const { entriesFromRows, rowsFromEntries, resultName, measureLabel, tileMeasureLabels } = await import("./chart-shape")
  const rows = rowsFromEntries([{ metric: "total_visitors" }, "rooms", { column: "stay", aggregate: "avg" }])
  assert.deepEqual(rows.map((r) => r.column), ["metric:total_visitors", "rooms", "stay"])
  // Own aggregates are on, yet the metric is sent by name alone.
  assert.deepEqual(entriesFromRows(rows, true), [{ metric: "total_visitors" }, "rooms", { column: "stay", aggregate: "avg" }])
  assert.equal(resultName(rows[0], new Set()), "total_visitors")
  assert.equal(measureLabel(rows[0], new Set()), "total_visitors")
  // A tile of a chart with a metric names its series by the metric, not "undefined of undefined".
  assert.deepEqual(tileMeasureLabels([{ metric: "total_visitors" }, { column: "stay", aggregate: "avg" }], ["total_visitors", "avg_stay"]), { avg_stay: "avg of stay" })
})

test("a measure left on the chart's aggregate says that aggregate, so an average is never drawn as the bare column (phase 2 QA R7)", () => {
  const none = new Set<string>()
  const plain = { column: "visitors", aggregate: "" }
  assert.equal(measureLabel(plain, none), "visitors")
  assert.equal(measureLabel(plain, none, "sum"), "visitors")
  assert.equal(measureLabel(plain, none, "avg"), "avg of visitors")
  assert.equal(measureLabel(plain, none, "count"), "count of visitors")
  // An aggregate of its own always reads, and a calculated aggregate or a metric keeps its name.
  assert.equal(measureLabel({ column: "visitors", aggregate: "max" }, none, "avg"), "max of visitors")
  assert.equal(measureLabel(plain, new Set(["visitors"]), "avg"), "visitors")
  assert.equal(measureLabel({ column: "metric:total", aggregate: "" }, none, "avg"), "total")
  assert.equal(effectiveAggregate(plain, "avg"), "avg")
  assert.equal(effectiveAggregate({ column: "x", aggregate: "min" }, "avg"), "min")
  assert.equal(effectiveAggregate(plain), "sum")
})

test("a tile reads a plain measure with the chart's aggregate, and keeps a measure's own", () => {
  assert.deepEqual(tileMeasureLabels(["visitors", { column: "stay", aggregate: "max" }], ["visitors", "max_stay"], undefined, undefined, "avg"), {
    visitors: "avg of visitors", max_stay: "max of stay",
  })
  assert.deepEqual(tileMeasureLabels(["visitors"], ["visitors"], undefined, undefined, "sum"), {})
  assert.deepEqual(tileMeasureLabels(["visitors"], ["visitors"]), {})
  // A definition saved before the aggregate was shown per measure reads the same way: the aggregate is the chart's.
  assert.deepEqual(tileMeasureLabels(["a", "b"], ["a", "b"], undefined, undefined, "count"), { a: "count of a", b: "count of b" })
})

test("a chart saved before is opened and saved back with the same measures and the same aggregate (phase 2 QA R4)", () => {
  type Saved = { measures: MeasureEntry[]; aggregate: string }
  const roundTrip = (saved: Saved): Saved => {
    // The builder keeps `aggregate` as the chart's and reads the measures as rows; an untouched row stores no aggregate of its own.
    const rows = rowsFromEntries(saved.measures)
    return { measures: entriesFromRows(rows, true), aggregate: saved.aggregate }
  }
  const olds: Saved[] = [
    { measures: ["visitors"], aggregate: "sum" },
    { measures: ["visitors", "stay"], aggregate: "avg" },
    { measures: ["visitors", { column: "stay", aggregate: "max" }], aggregate: "sum" },
    // An own aggregate equal to the chart's stays own, so its result name (`sum_visitors`) is the one the sort and the steps already use.
    { measures: [{ column: "visitors", aggregate: "sum" }], aggregate: "sum" },
    { measures: [{ metric: "total_visitors" }, "rooms"], aggregate: "count" },
  ]
  for (const old of olds) assert.deepEqual(roundTrip(old), old, JSON.stringify(old))
  // The statement is the same because the result names are the same.
  const rows = rowsFromEntries(["visitors", { column: "stay", aggregate: "avg" }])
  assert.deepEqual(rows.map((r) => resultName(r, new Set())), ["visitors", "avg_stay"])
})

test("a measure row that picks the chart's aggregate again stores none, so the result name does not change", () => {
  assert.equal(storedAggregate("sum", "sum"), "")
  assert.equal(resultName({ column: "visitors", aggregate: storedAggregate("sum", "sum") }, new Set()), "visitors")
  assert.equal(resultName({ column: "visitors", aggregate: storedAggregate("avg", "sum") }, new Set()), "avg_visitors")
})

test("columnRef: one reading of a column for pickers, summaries and labels (phase 2 QA R14)", () => {
  assert.deepEqual(columnRef("provinsi"), { text: "provinsi", tag: null, full: "provinsi" })
  assert.deepEqual(columnRef("mart_demo_province_capacity__rooms"), { text: "rooms", tag: "capacity", full: "mart_demo_province_capacity.rooms" })
  // A second join of the same table keeps its number, so the two stay apart.
  assert.deepEqual(columnRef("mart_demo_province_capacity2__rooms"), { text: "rooms", tag: "capacity2", full: "mart_demo_province_capacity2.rooms" })
  assert.deepEqual(columnRef("occupancy", { calculated: new Set(["occupancy"]) }), { text: "occupancy", tag: "fx", full: "occupancy" })
  assert.deepEqual(columnRef("band", { groups: new Set(["band"]) }), { text: "band", tag: "group", full: "band" })
  assert.deepEqual(columnRef("limit_n", { isParam: (n) => n === "limit_n" }), { text: "limit_n", tag: "param", full: "limit_n" })
  assert.deepEqual(columnRef(metricColumn("visitors_total"), { metricLabels: { visitors_total: "Total visitors" } }), { text: "Total visitors", tag: "metric", full: "Total visitors" })
  assert.equal(columnRef(metricColumn("visitors_total")).text, "visitors_total")
})

test("a measure row with a joined column reads 'max of rooms' (phase 2 QA R14, R16)", () => {
  assert.equal(measureLabel({ column: "mart_demo_province_capacity__rooms", aggregate: "max" }, new Set()), "max of rooms")
  assert.equal(measureLabel({ column: "mart_demo_province_capacity__rooms", aggregate: "" }, new Set()), "rooms")
  assert.equal(measureLabel({ column: "mart_demo_province_capacity__rooms", aggregate: "" }, new Set(), "avg"), "avg of rooms")
})
