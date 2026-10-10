import assert from "node:assert/strict"
import test from "node:test"

import { readKpi, signedAmount, signedPercent, sparklinePoints } from "./kpi-compare"

const prev = { kind: "previous", dateColumn: "day", period: "month" } as const
const rows = [{ day: "2026-01-01", v: "100" }, { day: "2026-02-01", v: "80" }, { day: "2026-03-01", v: "120" }]

test("the latest period is the value and the one before it the comparison", () => {
  const r = readKpi(rows, prev)
  assert.equal(r.value, 120)
  assert.equal(r.reference, 80)
  assert.equal(r.delta, 40)
  assert.equal(r.percent, 50)
  assert.deepEqual(r.series, [100, 80, 120])
  assert.equal(r.valueBucket, "2026-03-01")
  assert.equal(r.referenceBucket, "2026-02-01")
})

test("up is good by default and down can be good instead", () => {
  assert.equal(readKpi(rows, prev).tone, "good")
  assert.equal(readKpi(rows, prev, "down").tone, "bad")
  const falling = [{ day: "a", v: 10 }, { day: "b", v: 8 }]
  assert.equal(readKpi(falling, prev).tone, "bad")
  assert.equal(readKpi(falling, prev, "down").tone, "good")
  assert.equal(readKpi([{ day: "a", v: 5 }, { day: "b", v: 5 }], prev).tone, "flat")
})

test("a previous value of zero shows the amount and no percent", () => {
  const r = readKpi([{ day: "a", v: 0 }, { day: "b", v: 7 }], prev)
  assert.equal(r.delta, 7)
  assert.equal(r.percent, null)
  assert.equal(signedPercent(r.percent), "")
})

test("a single period has a value and nothing to compare with", () => {
  const r = readKpi([{ day: "a", v: 3 }], prev)
  assert.equal(r.value, 3)
  assert.equal(r.reference, null)
  assert.equal(r.delta, null)
  assert.equal(readKpi([], prev).value, null)
})

test("against a goal the distance and the percent of the goal, below or above", () => {
  const goal = { kind: "goal", value: 200 } as const
  const below = readKpi([{ v: "150" }], goal)
  assert.equal(below.delta, -50)
  assert.equal(below.percent, 75)
  assert.equal(below.tone, "bad")
  const above = readKpi([{ v: 250 }], goal)
  assert.equal(above.tone, "good")
  assert.equal(readKpi([{ v: 250 }], goal, "down").tone, "bad")
  assert.equal(readKpi([{ v: 200 }], goal).tone, "good")
  assert.equal(readKpi([{ v: 5 }], { kind: "goal", value: 0 }).percent, null)
})

test("without a comparison the number is the first row's v", () => {
  const r = readKpi([{ v: "42" }], undefined)
  assert.equal(r.value, 42)
  assert.equal(r.mode, "none")
})

test("amounts and percents carry their sign, and a flat line has no height", () => {
  assert.equal(signedPercent(12.34), "+12,3%")
  assert.equal(signedPercent(-3), "-3%")
  assert.equal(signedAmount(1234), "+1.234")
  assert.equal(signedAmount(-5), "-5")
  assert.equal(sparklinePoints([], 100, 20), "")
  assert.equal(sparklinePoints([4, 4], 100, 20), "2.0,10.0 98.0,10.0")
  assert.equal(sparklinePoints([0, 10], 100, 20), "2.0,18.0 98.0,2.0")
})
