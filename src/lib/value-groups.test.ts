import assert from "node:assert/strict"
import test from "node:test"

import { addGroup, groupOf, groupPayload, groupProblems, toggleValue, valueCount } from "./value-groups"

const java = { name: "Java", values: ["Jawa Barat", "Jawa Timur"] }
const sumatra = { name: "Sumatra", values: ["Aceh"] }

test("a value joins the active group, leaves it on a second click, and stays in one group only", () => {
  const added = toggleValue([java, sumatra], 1, "Riau")
  assert.deepEqual(added[1].values, ["Aceh", "Riau"])
  assert.deepEqual(toggleValue(added, 1, "Riau")[1].values, ["Aceh"])
  // Held by another group: the click changes nothing.
  assert.deepEqual(toggleValue([java, sumatra], 1, "Jawa Barat"), [java, sumatra])
  assert.equal(groupOf([java, sumatra], "Aceh"), 1)
  assert.equal(groupOf([java, sumatra], "Bali"), -1)
  assert.equal(valueCount([java, sumatra]), 3)
  // No group is active, or it is out of range.
  assert.deepEqual(toggleValue([java], -1, "Bali"), [java])
  assert.deepEqual(toggleValue([java], 4, "Bali"), [java])
})

test("the caps of 2,000 values and 50 groups stop an add", () => {
  const full = [{ name: "All", values: Array.from({ length: 2000 }, (_, i) => `v${i}`) }, { name: "B", values: [] }]
  assert.deepEqual(toggleValue(full, 1, "extra"), full)
  const fifty = Array.from({ length: 50 }, (_, i) => ({ name: `g${i}`, values: ["x"] }))
  assert.equal(addGroup(fifty).length, 50)
})

test("a new group is named after the count and never takes a name that is in use", () => {
  assert.deepEqual(addGroup([]), [{ name: "Group 1", values: [] }])
  assert.equal(addGroup([{ name: "Group 2", values: [] }])[1].name, "Group 3")
})

test("the editor lists what is still needed", () => {
  assert.deepEqual(groupProblems("", "", [], "keep"), ["a name for the field", "a column", "a group"])
  assert.deepEqual(groupProblems("region", "prov", [{ name: "A", values: [] }], "keep"), ["values in every group"])
  assert.deepEqual(groupProblems("region", "prov", [java, { ...sumatra, name: "Java" }], "keep"), ["different names for the groups"])
  assert.deepEqual(groupProblems("region", "prov", [{ ...java, name: "Other" }], "other"), ["no group called Other while the rest is called Other"])
  assert.deepEqual(groupProblems("region", "prov", [java, sumatra], "other"), [])
})

test("the payload trims names and keeps the values as they are spelled", () => {
  assert.deepEqual(groupPayload("prov", [{ name: " Java ", values: [" Jawa Barat"] }], "other"), {
    column: "prov", rest: "other", groups: [{ name: "Java", values: [" Jawa Barat"] }],
  })
})
