import assert from "node:assert/strict"
import test from "node:test"

import {
  coversKey, displayColumn, displayLabel, emptyJoin, joinAliases, joinComplete, joinDraftsFromDef, joinProblems,
  joinsPayload, leftChoices, multiplyWarning, pairsFromSuggestions, type JoinDraft,
} from "./chart-joins"
import { emptyShapeDraft, shapePayload, shapeProblems } from "./chart-shape"

const pair = (left: string, op: string, right: string) => ({ left, op, right })
const done = (mart: string, on = [pair("provinsi", "=", "provinsi")]): JoinDraft => ({ mart, type: "left", on })

test("a joined column reads table.column and a result column of the server's own is left alone", () => {
  assert.equal(displayColumn("capacity__rooms"), "capacity.rooms")
  assert.equal(displayColumn("capacity2__hotel_count"), "capacity2.hotel_count")
  assert.equal(displayColumn("provinsi"), "provinsi")
  assert.equal(displayColumn("__other"), "__other")
  assert.equal(displayColumn("trailing__"), "trailing__")
  assert.equal(displayLabel("avg of capacity__rooms"), "avg of capacity.rooms")
})

test("a join is sent only when every join has a table and complete pairs with one equals", () => {
  assert.deepEqual(joinsPayload([]), {})
  assert.deepEqual(joinsPayload([emptyJoin()]), {})
  assert.deepEqual(joinsPayload([done("capacity"), emptyJoin("x")]), {})
  const sent = joinsPayload([done("capacity")])
  assert.deepEqual(sent, { joins: [{ mart: "capacity", type: "left", on: [pair("provinsi", "=", "provinsi")] }] })
  assert.equal(joinComplete(done("c", [pair("a", "<", "b")])), false)
  assert.deepEqual(joinProblems([done("c", [pair("a", "<", "b")])]), ['Join 1 needs one "equals"'])
  assert.deepEqual(joinProblems([emptyJoin(), emptyJoin("c")]), ["Join 1 table", "Join 2 columns"])
})

test("the shape draft carries the joins into the payload and the problems, and a saved definition comes back as a draft", () => {
  const d = { ...emptyShapeDraft(), joins: [done("serving.capacity")] }
  const ctx = { kind: "bar", rowsMode: false, grained: false }
  assert.deepEqual((shapePayload(d, ctx).joins as unknown[]).length, 1)
  assert.equal(shapePayload(d, { ...ctx, kind: "text" }).joins, undefined)
  assert.deepEqual(shapeProblems({ ...d, joins: [emptyJoin()] }, ctx), ["Join 1 table"])
  const back = joinDraftsFromDef([{ mart: "serving.capacity", type: "full", on: [pair("a", "=", "b")] }])
  assert.deepEqual(back, [{ mart: "capacity", type: "full", on: [pair("a", "=", "b")] }])
})

test("the alias of a table joined twice gets a number, as the server gives it", () => {
  const cols = (m: string) => (m === "capacity" ? ["provinsi", "rooms"] : undefined)
  assert.deepEqual(joinAliases(["provinsi"], [done("capacity"), done("capacity")], cols), ["capacity", "capacity2"])
  // A base column that would collide with the first alias's column pushes the number on.
  assert.deepEqual(joinAliases(["capacity__rooms"], [done("capacity")], cols), ["capacity2"])
  // A table whose columns are not known yet keeps its plain name.
  assert.deepEqual(joinAliases([], [done("other")], cols), ["other"])
})

test("the left side of a join may name the base columns and those of the joins before it", () => {
  const cols = (m: string) => (m === "capacity" ? ["provinsi", "rooms"] : m === "cities" ? ["city"] : undefined)
  const joins = [done("capacity"), done("cities")]
  assert.deepEqual(leftChoices(["provinsi", "visitors"], joins, 0, cols), ["provinsi", "visitors"])
  assert.deepEqual(leftChoices(["provinsi", "visitors"], joins, 1, cols), ["provinsi", "visitors", "capacity__provinsi", "capacity__rooms"])
})

test("rows may multiply unless the equality pairs cover the joined table's sorting key", () => {
  assert.equal(coversKey(["provinsi"], [pair("p", "=", "provinsi")]), true)
  assert.equal(coversKey(["provinsi", "year"], [pair("p", "=", "provinsi")]), false)
  assert.equal(coversKey(["provinsi"], [pair("p", "<", "provinsi")]), false)
  assert.equal(coversKey([], [pair("p", "=", "provinsi")]), false)
  assert.equal(multiplyWarning(["provinsi"], done("c")), null)
  assert.match(multiplyWarning(["id"], done("c")) ?? "", /rows may multiply/)
  assert.equal(multiplyWarning(["id"], emptyJoin("c")), null)
})

test("the suggested pairs are applied as equals, and an empty suggestion leaves one blank pair", () => {
  const s = { left: "a", right: "b", rightKey: ["p"], coversRightKey: true, pairs: [
    { left: "p", right: "p", rank: "keys", suggested: true },
    { left: "d", right: "d", rank: "name_and_type", suggested: false },
  ] }
  assert.deepEqual(pairsFromSuggestions(s), [pair("p", "=", "p")])
  assert.deepEqual(pairsFromSuggestions({ ...s, pairs: [] }), [pair("", "=", "")])
})
