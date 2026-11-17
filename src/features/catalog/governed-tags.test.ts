import assert from "node:assert/strict"
import { test } from "bun:test"
import { chosenGovernedTags, governedPairLabels } from "./governed-tags"

test("governedPairLabels reads each pair as key: value, sorted by key", () => {
  assert.deepEqual(governedPairLabels({ tier: "gold", data_domain: "finance" }), [
    "data_domain: finance",
    "tier: gold",
  ])
})

test("governedPairLabels is empty for an asset without governed tags", () => {
  assert.deepEqual(governedPairLabels(undefined), [])
  assert.deepEqual(governedPairLabels(null), [])
  assert.deepEqual(governedPairLabels({}), [])
})

test("chosenGovernedTags leaves out a key left on none", () => {
  assert.deepEqual(chosenGovernedTags({ data_domain: "finance", tier: "" }), { data_domain: "finance" })
  assert.deepEqual(chosenGovernedTags({ tier: "" }), {})
})
