import assert from "node:assert/strict"
import { test } from "bun:test"
import { formatTagValues, parseTagValues } from "./tag-key-values"

test("parseTagValues splits on commas, spaces and new lines and drops empties", () => {
  assert.deepEqual(parseTagValues("finance, ops\nhr  ,, "), ["finance", "ops", "hr"])
  assert.deepEqual(parseTagValues("   "), [])
})

test("parseTagValues leaves a value the API would refuse exactly as typed", () => {
  assert.deepEqual(parseTagValues("Finance finance"), ["Finance", "finance"])
})

test("formatTagValues round-trips through parseTagValues", () => {
  const values = ["finance", "ops"]
  assert.deepEqual(parseTagValues(formatTagValues(values)), values)
})
