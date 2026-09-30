import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  decodeSourceChoice,
  encodeSourceChoice,
  fieldsQuery,
  sourcePayload,
  sourceValueFromDef,
} from "./chart-source"

test("a mart and a SQL source round-trip through the select value", () => {
  for (const choice of [
    { kind: "mart", name: "mart_material_by_group" },
    { kind: "sql", id: "s_1234abcd" },
  ] as const) {
    assert.deepEqual(decodeSourceChoice(encodeSourceChoice(choice)), choice)
  }
})

test("empty, bare-prefix and unknown values decode to null", () => {
  for (const v of ["", null, undefined, "mart:", "sql:", "mart_x", "table:x"]) {
    assert.equal(decodeSourceChoice(v), null)
  }
})

test("the payload carries exactly one of mart or sqlSource", () => {
  assert.deepEqual(sourcePayload({ kind: "mart", name: "m" }), { mart: "m" })
  assert.deepEqual(sourcePayload({ kind: "sql", id: "s_1" }), { sqlSource: "s_1" })
  assert.deepEqual(sourcePayload(null), {})
})

test("a stored definition prefers its SQL source over an empty mart", () => {
  assert.equal(sourceValueFromDef({ mart: "", sqlSource: "s_1" }), "sql:s_1")
  assert.equal(sourceValueFromDef({ mart: "m" }), "mart:m")
  assert.equal(sourceValueFromDef({}), "")
})

test("the fields query names the right parameter and escapes it", () => {
  assert.equal(fieldsQuery({ kind: "mart", name: "a b" }), "mart=a%20b")
  assert.equal(fieldsQuery({ kind: "sql", id: "s_1" }), "source=s_1")
})
