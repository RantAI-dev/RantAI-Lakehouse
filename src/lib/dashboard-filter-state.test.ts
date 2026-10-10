import { strict as assert } from "node:assert"
import { test } from "node:test"
import type { FilterDef } from "@/services/clients/bi-store"
import {
  datePickToFilter, decodeFilters, defaultOp, dropInertFilters, enforceRequired, isRequiredColumn, encodeFilters, filterLabel, filterToDatePick, filterToNumberPick, filtersEqual,
  filtersToParam, isActiveFilter, normalizeFilters, numberPickToFilter, toggleValue, type DatePick, type NumberPick,
} from "./dashboard-filter-state"

const all: FilterDef[] = [
  { column: "province", values: ["Bali", "Aceh"] },
  { column: "city", op: "not_in", values: ["Ubud"] },
  { column: "price", op: "between", values: [], min: "100" },
  { column: "date", op: "relative", values: [], anchor: "last", n: 30, unit: "day" },
  { column: "name", op: "contains", values: [], text: "50%" },
]

test("encoding then decoding gives back the same active filters", () => {
  const back = decodeFilters(encodeFilters(all))
  assert.ok(back)
  assert.ok(filtersEqual(back, all))
  assert.deepEqual(back, normalizeFilters(all))
})

test("nothing active encodes to an empty parameter", () => {
  assert.equal(encodeFilters([]), "")
  assert.equal(encodeFilters([{ column: "c", values: [] }, { column: "d", op: "between", values: [] }]), "")
})

test("a link that is not a filter list decodes to null, not to a half state", () => {
  for (const bad of ["", "nope", "{}", '[{"values":[]}]', '[{"column":"c","op":"regex"}]', "[1]"]) {
    assert.equal(decodeFilters(bad), null, bad)
  }
  assert.equal(decodeFilters(null), null)
})

test("equality ignores filter order, value order and empty placeholders", () => {
  const a: FilterDef[] = [{ column: "a", values: ["x", "y"] }, { column: "b", values: ["z"] }]
  const b: FilterDef[] = [{ column: "b", values: ["z"] }, { column: "a", values: ["y", "x"] }, { column: "c", values: [] }]
  assert.ok(filtersEqual(a, b))
  assert.ok(!filtersEqual(a, [{ column: "a", values: ["x"] }, { column: "b", values: ["z"] }]))
  // `op: "in"` written out is the same filter as the legacy shape.
  assert.ok(filtersEqual([{ column: "a", op: "in", values: ["x"] }], [{ column: "a", values: ["x"] }]))
})

test("a filter is active only when it restricts something", () => {
  assert.ok(!isActiveFilter({ column: "c", values: [] }))
  assert.ok(!isActiveFilter({ column: "c", op: "relative", values: [], unit: "day", anchor: "last" }))
  assert.ok(!isActiveFilter({ column: "c", op: "contains", values: [], text: "" }))
  assert.ok(isActiveFilter({ column: "c", op: "between", values: [], max: "5" }))
})

test("chip labels read in words for every op", () => {
  assert.equal(filterLabel(all[0]), "province is Bali, Aceh")
  assert.equal(filterLabel(all[1]), "city is not Ubud")
  assert.equal(filterLabel(all[2]), "price ≥ 100")
  assert.equal(filterLabel(all[3]), "date in the last 30 days")
  assert.equal(filterLabel(all[4]), 'name contains "50%"')
  assert.equal(filterLabel({ column: "p", op: "between", values: [], min: "1", max: "9" }), "p between 1 and 9")
  assert.equal(filterLabel({ column: "p", op: "between", values: [], max: "9" }), "p ≤ 9")
  assert.equal(filterLabel({ column: "d", op: "between", values: [], min: "2024-01-01", max: "2024-02-01" }), "d from 2024-01-01 to 2024-02-01")
  assert.equal(filterLabel({ column: "d", op: "between", values: [], min: "2024-01-01" }), "d from 2024-01-01")
  assert.equal(filterLabel({ column: "d", op: "relative", values: [], anchor: "this", unit: "month" }), "d in this month")
  assert.equal(filterLabel({ column: "d", op: "relative", values: [], anchor: "previous", unit: "year" }), "d in the previous year")
  assert.equal(filterLabel({ column: "d", op: "relative", values: [], anchor: "last", n: 1, unit: "week" }), "d in the last week")
  assert.equal(filterLabel({ column: "c", values: ["a", "b", "c", "d", "e"] }), "c is a, b, c +2")
  assert.equal(filterLabel({ column: "c", op: "starts_with", values: [], text: "b" }), 'c starts with "b"')
})

test("the starting op follows the column kind", () => {
  assert.equal(defaultOp("text"), "in")
  assert.equal(defaultOp("number"), "between")
  assert.equal(defaultOp("date"), "relative")
  assert.equal(defaultOp("datetime"), "relative")
})

test("clicking a chart value toggles it in the value filter of that column", () => {
  const one = toggleValue([], "region", "north")
  assert.deepEqual(one, [{ column: "region", values: ["north"] }])
  const two = toggleValue(one, "region", "south")
  assert.deepEqual(two[0].values, ["north", "south"])
  assert.deepEqual(toggleValue(two, "region", "north")[0].values, ["south"])
  assert.deepEqual(toggleValue(one, "region", "north"), [])
  // A range on the same column is left alone; the click adds a value filter beside it.
  const withRange: FilterDef[] = [{ column: "region", op: "not_in", values: ["x"] }]
  assert.equal(toggleValue(withRange, "region", "n").length, 2)
})

test("the address carries the state only when it differs from the saved default", () => {
  const saved: FilterDef[] = [{ column: "region", values: ["north"] }]
  assert.equal(filtersToParam([{ column: "region", values: ["north"] }], saved), null)
  assert.equal(filtersToParam([], saved), "[]")
  assert.deepEqual(decodeFilters(filtersToParam([], saved)), [])
  assert.equal(filtersToParam([], []), null)
  const changed = filtersToParam([{ column: "region", values: ["south"] }], saved)
  assert.deepEqual(decodeFilters(changed), [{ column: "region", values: ["south"] }])
})

// ---- BI-18 part A, second round: the named comparisons --------------------

test("a round-one link and a round-one saved board decode to the same filters and labels", () => {
  // Taken from the wire form round one wrote (`filters` JSON on the board, `f` in the address).
  const roundOne =
    '[{"column":"province","values":["Bali","Aceh"]},{"column":"price","values":[],"op":"between","min":"100"},' +
    '{"column":"date","values":[],"op":"relative","unit":"day","n":30,"anchor":"last"},' +
    '{"column":"d","values":[],"op":"between","min":"2024-01-01","max":"2024-02-01"},' +
    '{"column":"name","values":[],"op":"contains","text":"50%"}]'
  const back = decodeFilters(roundOne)
  assert.ok(back)
  assert.deepEqual(back, JSON.parse(roundOne))
  assert.deepEqual(back.map((f) => filterLabel(f)), [
    "province is Bali, Aceh", "price ≥ 100", "date in the last 30 days", "d from 2024-01-01 to 2024-02-01", 'name contains "50%"',
  ])
  // Encoding adds none of the new fields to a round-one filter.
  assert.ok(!encodeFilters(back).includes("Exclusive"))
  assert.ok(!encodeFilters(back).includes("required"))
})

const datePicks: [DatePick, Partial<FilterDef>, string][] = [
  [{ mode: "last", n: 7, unit: "day" }, { op: "relative", anchor: "last", n: 7, unit: "day" }, "d in the last 7 days"],
  [{ mode: "next", n: 7, unit: "day" }, { op: "relative", anchor: "next", n: 7, unit: "day" }, "d in the next 7 days"],
  [{ mode: "next", n: 1, unit: "month" }, { op: "relative", anchor: "next", n: 1, unit: "month" }, "d in the next month"],
  [{ mode: "this", unit: "quarter" }, { op: "relative", anchor: "this", unit: "quarter" }, "d in this quarter"],
  [{ mode: "previous", unit: "year" }, { op: "relative", anchor: "previous", unit: "year" }, "d in the previous year"],
  [{ mode: "on", date: "2026-10-03" }, { op: "between", min: "2026-10-03", max: "2026-10-03" }, "d on 3 Oct 2026"],
  [{ mode: "before", date: "2026-10-03" }, { op: "between", max: "2026-10-03", maxExclusive: true }, "d before 3 Oct 2026"],
  [{ mode: "after", date: "2026-10-03" }, { op: "between", min: "2026-10-03", minExclusive: true }, "d after 3 Oct 2026"],
  [{ mode: "range", from: "2026-01-05", to: "2026-02-01" }, { op: "between", min: "2026-01-05", max: "2026-02-01" }, "d from 2026-01-05 to 2026-02-01"],
  [{ mode: "range", from: "2026-01-05" }, { op: "between", min: "2026-01-05" }, "d from 2026-01-05"],
  [{ mode: "range", to: "2026-01-05" }, { op: "between", max: "2026-01-05" }, "d until 2026-01-05"],
  [{ mode: "month", year: 2026, month: 3 }, { op: "between", min: "2026-03-01", max: "2026-03-31" }, "d in March 2026"],
  [{ mode: "month", year: 2024, month: 2 }, { op: "between", min: "2024-02-01", max: "2024-02-29" }, "d in February 2024"],
  [{ mode: "quarter", year: 2026, quarter: 1 }, { op: "between", min: "2026-01-01", max: "2026-03-31" }, "d in Q1 2026"],
  [{ mode: "quarter", year: 2026, quarter: 4 }, { op: "between", min: "2026-10-01", max: "2026-12-31" }, "d in Q4 2026"],
]

test("every date pick is stored as the plan says, reads back as itself and has its label", () => {
  for (const [pick, stored, label] of datePicks) {
    const f = datePickToFilter("d", pick)
    assert.ok(f, JSON.stringify(pick))
    assert.deepEqual(f, { column: "d", values: [], ...stored }, JSON.stringify(pick))
    assert.deepEqual(filterToDatePick(f), pick, JSON.stringify(pick))
    assert.equal(filterLabel(f), label)
    // It survives the address.
    const back = decodeFilters(encodeFilters([f]))
    assert.ok(back && filtersEqual(back, [f]))
    assert.deepEqual(filterToDatePick(back[0]), pick)
  }
})

test("an incomplete or impossible date pick stores nothing", () => {
  for (const pick of [
    { mode: "on", date: "" }, { mode: "on", date: "2026-02-30" }, { mode: "before", date: "2026-1-1" }, { mode: "after", date: "1899-12-31" },
    { mode: "range" }, { mode: "range", from: "2026-02-02", to: "2026-02-01" },
    { mode: "last", n: 0, unit: "day" }, { mode: "next", n: 3651, unit: "day" }, { mode: "next", n: 1.5, unit: "day" },
    { mode: "month", year: 2026, month: 13 }, { mode: "month", year: 1800, month: 1 }, { mode: "quarter", year: 2026, quarter: 5 },
  ] as DatePick[]) {
    assert.equal(datePickToFilter("d", pick), null, JSON.stringify(pick))
  }
})

test("a calendar month or quarter written as a range reads as the month or quarter, and others stay ranges", () => {
  const range = (min: string, max: string): FilterDef => ({ column: "d", op: "between", values: [], min, max })
  assert.deepEqual(filterToDatePick(range("2026-03-01", "2026-03-31")), { mode: "month", year: 2026, month: 3 })
  assert.deepEqual(filterToDatePick(range("2026-04-01", "2026-06-30")), { mode: "quarter", year: 2026, quarter: 2 })
  assert.deepEqual(filterToDatePick(range("2026-03-01", "2026-03-30")), { mode: "range", from: "2026-03-01", to: "2026-03-30" })
  assert.deepEqual(filterToDatePick(range("2026-02-01", "2026-04-30")), { mode: "range", from: "2026-02-01", to: "2026-04-30" })
  assert.equal(filterLabel(range("2026-03-01", "2026-03-30")), "d from 2026-03-01 to 2026-03-30")
  // Exclusive ends are never a whole month.
  assert.equal(filterToDatePick({ ...range("2026-03-01", "2026-03-31"), maxExclusive: true }), null)
})

const numberPicks: [NumberPick, Partial<FilterDef>, string][] = [
  [{ mode: "eq", value: "5" }, { op: "between", min: "5", max: "5" }, "n = 5"],
  [{ mode: "ne", value: "5" }, { op: "not_in", values: ["5"] }, "n ≠ 5"],
  [{ mode: "gt", value: "5" }, { op: "between", min: "5", minExclusive: true }, "n > 5"],
  [{ mode: "lt", value: "2.5" }, { op: "between", max: "2.5", maxExclusive: true }, "n < 2.5"],
  [{ mode: "between", min: "1", max: "9" }, { op: "between", min: "1", max: "9" }, "n between 1 and 9"],
  [{ mode: "between", min: "100" }, { op: "between", min: "100" }, "n ≥ 100"],
  [{ mode: "between", max: "-3" }, { op: "between", max: "-3" }, "n ≤ -3"],
]

test("every number pick is stored as the plan says, reads back as itself and has its label", () => {
  for (const [pick, stored, label] of numberPicks) {
    const f = numberPickToFilter("n", pick)
    assert.ok(f, JSON.stringify(pick))
    const wire = { column: "n", values: [], ...stored }
    assert.deepEqual(f, wire, JSON.stringify(pick))
    assert.deepEqual(filterToNumberPick(f), pick, JSON.stringify(pick))
    assert.equal(filterLabel(f, "number"), label)
    const back = decodeFilters(encodeFilters([f]))
    assert.ok(back && filtersEqual(back, [f]))
  }
})

test("an incomplete or non-numeric number pick stores nothing", () => {
  for (const pick of [
    { mode: "eq", value: "" }, { mode: "gt", value: "abc" }, { mode: "lt", value: "NaN" }, { mode: "ne", value: "1e999" },
    { mode: "between" }, { mode: "between", min: "x" }, { mode: "between", min: "9", max: "1" },
  ] as NumberPick[]) {
    assert.equal(numberPickToFilter("n", pick), null, JSON.stringify(pick))
  }
})

test("not equal reads as a number comparison only on a number column, and a value list is not a pick", () => {
  const ne: FilterDef = { column: "c", op: "not_in", values: ["5"] }
  assert.equal(filterLabel(ne), "c is not 5")
  assert.equal(filterLabel(ne, "text"), "c is not 5")
  assert.equal(filterLabel(ne, "number"), "c ≠ 5")
  assert.equal(filterToNumberPick({ column: "c", values: ["5"] }), null)
  assert.equal(filterToNumberPick({ column: "c", op: "not_in", values: ["5", "6"] }), null)
})

test("does-not-contain is a text filter that survives the address and reads in words", () => {
  const f: FilterDef = { column: "name", op: "not_contains", values: [], text: "test" }
  assert.equal(filterLabel(f), 'name does not contain "test"')
  assert.ok(isActiveFilter(f))
  assert.ok(!isActiveFilter({ ...f, text: "" }))
  const back = decodeFilters(encodeFilters([f]))
  assert.deepEqual(back, [f])
})

test("exclusive ends and required count when comparing with the saved default", () => {
  const inclusive: FilterDef = { column: "n", op: "between", values: [], min: "5" }
  assert.ok(!filtersEqual([inclusive], [{ ...inclusive, minExclusive: true }]))
  assert.ok(!filtersEqual([inclusive], [{ ...inclusive, required: true }]))
  // `false` written out is the same filter as absent, and is not kept on the wire.
  assert.ok(filtersEqual([inclusive], [{ ...inclusive, minExclusive: false, required: false }]))
  assert.ok(!encodeFilters([{ ...inclusive, minExclusive: false, required: false }]).includes("xclusive"))
  // An exclusive flag with no bound to exclude means nothing.
  assert.ok(filtersEqual([inclusive], [{ ...inclusive, maxExclusive: true }]))
})

// ---- required filters ------------------------------------------------------

const requiredDefault: FilterDef[] = [
  { column: "region", values: ["north"], required: true },
  { column: "year", op: "between", values: [], min: "2025", required: true },
  { column: "city", values: ["Ubud"] },
]

test("a link that omits a required column gets the saved default's filter for it", () => {
  const fromLink: FilterDef[] = [{ column: "city", values: ["Ubud"] }]
  const out = enforceRequired(fromLink, requiredDefault)
  assert.deepEqual(out.map((f) => f.column).sort(), ["city", "region", "year"])
  assert.deepEqual(out.find((f) => f.column === "region"), requiredDefault[0])
  // `f=[]` (everything cleared) is the same case.
  assert.equal(enforceRequired([], requiredDefault).length, 2)
})

test("clearing or removing a required filter restores the default's value, other columns stay cleared", () => {
  const state: FilterDef[] = [{ column: "region", values: ["south"], required: true }, { column: "city", values: ["Ubud"] }]
  // The person emptied the region value list (an inactive placeholder) and removed the year.
  const cleared = state.map((f) => (f.column === "region" ? { ...f, values: [] } : f))
  const out = enforceRequired(cleared, requiredDefault)
  assert.deepEqual(out.find((f) => f.column === "region"), requiredDefault[0])
  assert.ok(out.some((f) => f.column === "year"))
  assert.equal(out.filter((f) => f.column === "region").length, 1, "the placeholder is replaced, not kept beside it")
  // Removing the non-required one is respected.
  const noCity = enforceRequired(state.filter((f) => f.column !== "city"), requiredDefault)
  assert.ok(!noCity.some((f) => f.column === "city"))
})

test("a required column the person changed keeps the person's value", () => {
  const state: FilterDef[] = [{ column: "region", values: ["south"] }, { column: "year", op: "between", values: [], min: "2020" }]
  const out = enforceRequired(state, requiredDefault)
  assert.equal(out, state, "nothing missing: the same list comes back")
  assert.deepEqual(out[0].values, ["south"])
})

test("Reset (the saved default itself) is untouched by enforcement", () => {
  assert.equal(enforceRequired(requiredDefault, requiredDefault), requiredDefault)
  assert.equal(filtersToParam(enforceRequired(requiredDefault, requiredDefault), requiredDefault), null)
})

test("a default that no longer requires a column stops enforcing it, and an inactive requirement is not one", () => {
  const relaxed = requiredDefault.map((f) => ({ ...f, required: undefined }))
  assert.deepEqual(enforceRequired([], relaxed), [])
  assert.ok(!isRequiredColumn("region", relaxed))
  assert.ok(isRequiredColumn("region", requiredDefault))
  assert.ok(!isRequiredColumn("city", requiredDefault))
  // A required placeholder with no value restricts nothing, so it cannot be demanded.
  assert.deepEqual(enforceRequired([], [{ column: "region", values: [], required: true }]), [])
})

test("the required flag survives the address and is part of the comparison with the default", () => {
  const back = decodeFilters(encodeFilters(requiredDefault))
  assert.ok(back && filtersEqual(back, requiredDefault))
  assert.ok(!filtersEqual(requiredDefault, requiredDefault.map((f) => ({ ...f, required: undefined }))))
})

// ---- filters that filter nothing (review round 1) --------------------------

test("the payload the pre-BI-18 bar left behind yields no filters, is the same as no default, and is not dirty", () => {
  const stored = JSON.parse('[{"column":"visit_date","values":[]},{"column":"visitors","values":[]}]') as FilterDef[]
  assert.deepEqual(dropInertFilters(stored), [])
  assert.ok(filtersEqual(stored, []))
  assert.equal(filtersToParam(dropInertFilters(stored), dropInertFilters(stored)), null)
  assert.equal(filtersToParam([], stored), null, "no filters equals a default made only of placeholders")
  // A link carrying them decodes to the empty state, not to two phantom columns.
  assert.deepEqual(decodeFilters(JSON.stringify(stored)), [])
})

test("only filters that restrict something survive, in their order, whatever their op", () => {
  const keep: FilterDef = { column: "k", values: ["a"] }
  const mixed: FilterDef[] = [
    { column: "a", values: [] }, { column: "b", op: "not_in", values: [] }, keep,
    { column: "c", op: "between", values: [] }, { column: "d", op: "contains", values: [], text: "" },
    { column: "e", op: "relative", values: [], unit: "day", anchor: "last" },
  ]
  assert.deepEqual(dropInertFilters(mixed), [keep])
})

test("a required placeholder with no value is dropped and does not make its column required", () => {
  const saved: FilterDef[] = [{ column: "region", values: [], required: true }]
  assert.deepEqual(dropInertFilters(saved), [])
  assert.ok(!isRequiredColumn("region", dropInertFilters(saved)))
  assert.deepEqual(enforceRequired([], dropInertFilters(saved)), [])
})
