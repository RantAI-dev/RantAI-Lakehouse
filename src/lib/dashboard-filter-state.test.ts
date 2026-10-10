import { strict as assert } from "node:assert"
import { test } from "node:test"
import type { FilterDef } from "@/services/clients/bi-store"
import {
  decodeFilters, defaultOp, encodeFilters, filterLabel, filtersEqual, filtersToParam, isActiveFilter, normalizeFilters, toggleValue,
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
