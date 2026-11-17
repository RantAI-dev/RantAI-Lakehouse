import assert from "node:assert/strict"
import test from "node:test"

import { listTimeZones } from "./time-zones"

test("the list always has UTC and the saved zone, sorted and without repeats", () => {
  const zones = listTimeZones("Mars/Olympus")
  assert.ok(zones.includes("UTC"))
  assert.ok(zones.includes("Mars/Olympus"))
  assert.equal(new Set(zones).size, zones.length)
  assert.deepEqual(zones, [...zones].sort((a, b) => a.localeCompare(b)))
})

test("the zone the deployment starts with is offered", () => {
  assert.ok(listTimeZones().includes("Asia/Jakarta"))
})
