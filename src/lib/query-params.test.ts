import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  isParamValue,
  isQueryParamType,
  parseDefinitions,
  parseValues,
  valueFitsType,
} from "./query-params"

test("isQueryParamType accepts the six types the server knows and nothing else", () => {
  for (const type of ["text", "number", "date", "dateRange", "dropdown", "multiSelect"]) {
    assert.equal(isQueryParamType(type), true, type)
  }
  for (const other of ["date_range", "DateRange", "list", "", 3, null, undefined, {}]) {
    assert.equal(isQueryParamType(other), false, String(other))
  }
})

test("valueFitsType reads each type's wire shape", () => {
  assert.equal(valueFitsType("text", "EU"), true)
  assert.equal(valueFitsType("text", 3), false)
  assert.equal(valueFitsType("number", 3), true)
  assert.equal(valueFitsType("number", Number.NaN), false)
  assert.equal(valueFitsType("number", "3"), false)
  assert.equal(valueFitsType("date", "2026-11-07"), true)
  assert.equal(valueFitsType("date", "7 Nov 2026"), false)
  assert.equal(valueFitsType("dateRange", { start: "2026-11-01", end: "2026-11-07" }), true)
  assert.equal(valueFitsType("dateRange", { start: "2026-11-01" }), false)
  assert.equal(
    valueFitsType("dateRange", { start: "2026-11-01", end: "2026-11-07", extra: "x" }),
    false
  )
  assert.equal(valueFitsType("dateRange", "2026-11-01"), false)
  assert.equal(valueFitsType("dropdown", "eu"), true)
  assert.equal(valueFitsType("dropdown", 5), true)
  assert.equal(valueFitsType("dropdown", ["eu"]), false)
  assert.equal(valueFitsType("multiSelect", ["eu", "us"]), true)
  assert.equal(valueFitsType("multiSelect", [1, 2]), true)
  assert.equal(valueFitsType("multiSelect", []), false)
  assert.equal(valueFitsType("multiSelect", "eu"), false)
  assert.equal(valueFitsType("multiSelect", [{}]), false)
})

test("isParamValue accepts any of the wire shapes and rejects the rest", () => {
  assert.equal(isParamValue("a"), true)
  assert.equal(isParamValue(1), true)
  assert.equal(isParamValue({ start: "2026-11-01", end: "2026-11-07" }), true)
  assert.equal(isParamValue(["a"]), true)
  for (const bad of [null, undefined, true, [], {}, { start: "2026-11-01" }, Infinity]) {
    assert.equal(isParamValue(bad), false, String(bad))
  }
})

test("parseDefinitions keeps a valid definition of every type as it is", () => {
  const stored = [
    { name: "who", type: "text", default: "me" },
    { name: "n", type: "number", default: 3 },
    { name: "day", type: "date", default: "2026-11-07" },
    { name: "span", type: "dateRange", default: { start: "2026-11-01", end: "2026-11-07" } },
    {
      name: "region",
      type: "dropdown",
      default: "eu",
      options: [
        { value: "eu", label: "Europe" },
        { value: "us", label: "United States" },
      ],
    },
    {
      name: "ids",
      type: "multiSelect",
      default: ["a"],
      options: [
        { value: "a", label: "A" },
        { value: "b", label: "B" },
      ],
    },
  ]
  assert.deepEqual(parseDefinitions(stored), stored)
})

test("parseDefinitions leaves default and options out when the definition has none", () => {
  const [def] = parseDefinitions([{ name: "who", type: "text" }])
  assert.deepEqual(def, { name: "who", type: "text" })
  assert.equal("default" in def, false)
  assert.equal("options" in def, false)
})

test("parseDefinitions drops a malformed definition and keeps its neighbours", () => {
  const good = { name: "who", type: "text", default: "me" }
  const stored = [
    good,
    { name: "", type: "text" },
    { name: 7, type: "text" },
    { name: "kind", type: "list" },
    { name: "noType" },
    "not a definition",
    null,
    { name: "bad_option", type: "dropdown", options: [{ value: "a" }] },
    { name: "bad_options", type: "dropdown", options: "a,b" },
    { name: "bad_option_value", type: "dropdown", options: [{ value: {}, label: "A" }] },
  ]
  assert.deepEqual(parseDefinitions(stored), [good])
})

test("parseDefinitions drops a default that does not fit the type and keeps the definition", () => {
  const defs = parseDefinitions([
    { name: "n", type: "number", default: "three" },
    { name: "span", type: "dateRange", default: "2026-11-01" },
    { name: "ids", type: "multiSelect", default: [], options: [{ value: "a", label: "A" }] },
  ])
  assert.deepEqual(defs, [
    { name: "n", type: "number" },
    { name: "span", type: "dateRange" },
    { name: "ids", type: "multiSelect", options: [{ value: "a", label: "A" }] },
  ])
})

test("parseDefinitions keeps the first of two definitions that share a name", () => {
  const defs = parseDefinitions([
    { name: "who", type: "text", default: "first" },
    { name: "who", type: "number", default: 2 },
  ])
  assert.deepEqual(defs, [{ name: "who", type: "text", default: "first" }])
})

test("parseDefinitions ignores a stored value that is not a list", () => {
  assert.deepEqual(parseDefinitions(undefined), [])
  assert.deepEqual(parseDefinitions("text"), [])
  assert.deepEqual(parseDefinitions({ name: "who", type: "text" }), [])
})

test("parseDefinitions drops a field it does not know", () => {
  const defs = parseDefinitions([{ name: "who", type: "text", colour: "red" }])
  assert.deepEqual(defs, [{ name: "who", type: "text" }])
})

test("parseValues drops a value of the wrong shape for its definition and keeps the rest", () => {
  const defs = parseDefinitions([
    { name: "who", type: "text" },
    { name: "n", type: "number" },
  ])
  assert.deepEqual(parseValues({ who: "me", n: "three" }, defs), { who: "me" })
})

test("parseValues keeps a value whose definition is gone when it has a valid shape", () => {
  assert.deepEqual(parseValues({ gone: "x", bad: null, worse: {} }, []), { gone: "x" })
})

test("parseValues reads a stored __proto__ key as a plain name", () => {
  const values = parseValues(JSON.parse('{"__proto__": "x", "who": "me"}'), [])
  assert.equal(Object.getPrototypeOf(values), Object.prototype)
  assert.deepEqual(Object.keys(values).sort(), ["__proto__", "who"])
})

test("parseValues ignores a stored value that is not an object", () => {
  assert.deepEqual(parseValues(undefined, []), {})
  assert.deepEqual(parseValues(["a"], []), {})
  assert.deepEqual(parseValues("a", []), {})
})
