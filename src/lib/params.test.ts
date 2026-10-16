import assert from "node:assert/strict"
import test from "node:test"

import {
  columnSwitchParams, limitParam, normalizeDef, paramsDiffer, paramsQuery, readParamsParam, refLinesFromDef, refLinesPayload,
  savedParamsBody, slotParam, slotText, withParamsParam, withValue, type ParamControl, type ParamDef,
} from "./params"

const def = (name: string, type: string, extra: Partial<ParamDef> = {}): ParamDef => ({ id: `p_${name}`, name, label: name, type, default: 1, ...extra })
const control = (name: string, value: string | number): ParamControl => ({ param: def(name, "number"), value })

test("a parameter slot is the text {name} and nothing else is", () => {
  assert.equal(slotText("pick"), "{pick}")
  assert.equal(slotParam("{pick}"), "pick")
  for (const t of ["pick", "{}", "{a b}", "{1a}", "", null, undefined]) assert.equal(slotParam(t as string), null)
})

test("the server's {param} objects become slot text where a definition enters the console", () => {
  const got = normalizeDef({
    dimension: { param: "dim" }, breakdown: "b", limit: { param: "n" },
    measures: ["visitors", { column: { param: "m" } }, { column: { param: "m2" }, aggregate: "avg" }, { column: "x", aggregate: "sum" }],
  })
  assert.equal(got?.dimension, "{dim}")
  assert.equal(got?.breakdown, "b")
  assert.deepEqual(got?.measures, ["visitors", "{m}", { column: "{m2}", aggregate: "avg" }, { column: "x", aggregate: "sum" }])
  assert.equal(limitParam(got?.limit), "n")
  assert.equal(limitParam(20), null)
})

test("a definition without parameters comes back equal", () => {
  const plain = { dimension: "a", measures: ["b"], limit: 5 }
  assert.deepEqual(normalizeDef(plain), plain)
  assert.equal(normalizeDef(undefined), undefined)
})

test("only an object of text and numbers in the address is a choice", () => {
  assert.deepEqual(readParamsParam('{"target":500,"pick":"rooms"}'), { target: 500, pick: "rooms" })
  for (const bad of [null, "", "[1]", "nope", '"x"', '{"a":{"b":1}}', "{}"]) assert.equal(readParamsParam(bad), null)
})

test("values are written to and removed from the address", () => {
  assert.equal(withParamsParam("f=1", { target: 5 }), "f=1&p=%7B%22target%22%3A5%7D")
  assert.equal(withParamsParam("f=1&p=%7B%7D", null), "f=1")
  assert.equal(paramsQuery(null), null)
  assert.equal(paramsQuery({ a: 1 }), '{"a":1}')
})

test("a value equal to the saved one is dropped, so the address holds only differences", () => {
  assert.deepEqual(withValue(null, { target: 1000 }, "target", 500), { target: 500 })
  assert.equal(withValue({ target: 500 }, { target: 1000 }, "target", "1000"), null)
  assert.deepEqual(withValue({ a: 1, b: 2 }, { a: 1 }, "b", 3), { a: 1, b: 3 })
})

test("Save as default is offered only when a shown value differs from the saved one", () => {
  const controls = [control("target", 500)]
  assert.equal(paramsDiffer(controls, { target: 1000 }), true)
  assert.equal(paramsDiffer(controls, { target: 500 }), false)
  assert.equal(paramsDiffer([control("target", 1)], undefined), false)
  assert.deepEqual(savedParamsBody(controls), [{ name: "target", value: 500 }])
})

test("only an all-text list can switch a column", () => {
  const text = def("a", "list", { values: [{ value: "x", label: "x" }] })
  const nums = def("b", "list", { values: [{ value: 1, label: "1" }] })
  assert.deepEqual(columnSwitchParams([text, nums, def("c", "number")]).map((d) => d.name), ["a"])
})

test("reference lines round trip, and a row with neither a number nor a parameter is dropped", () => {
  const lines = [{ value: 5, label: "Goal" }, { value: { param: "target" } }]
  assert.deepEqual(refLinesPayload(refLinesFromDef(lines)), lines)
  assert.deepEqual(refLinesPayload([{ value: "", param: "", label: "x" }, { value: "abc", param: "", label: "" }]), [])
  const six = Array.from({ length: 6 }, (_, i) => ({ value: String(i), param: "", label: "" }))
  assert.equal(refLinesPayload(six).length, 5)
})
