import { strict as assert } from "node:assert"
import { test } from "node:test"
import reference from "./forecast-reference.json"
import { forecast, isRefusal, seasonLength, type Forecast } from "./forecast"

// BI-17 part B, T2. Reference numbers: ops/forecast/make_reference.py
// (statsmodels ETSModel, additive error, heuristic start).

const ok = (r: ReturnType<typeof forecast>): Forecast => {
  assert.ok(!isRefusal(r), isRefusal(r) ? r.refused : "")
  return r
}
const refusal = (r: ReturnType<typeof forecast>): string => {
  assert.ok(isRefusal(r), "expected a refusal")
  return isRefusal(r) ? r.refused : ""
}
type Ref = { mean: number[]; lower: number[]; upper: number[] }
const relative = (got: number, want: number) => Math.abs(got - want) / Math.abs(want)

for (const [name, grain] of [["airline", "month"], ["gdp", "quarter"]] as const) {
  const ref = reference[name]
  const result = () => ok(forecast({ values: ref.values.map(Math.log), grain, horizon: ref.horizon }))

  test(`${name}: the model chosen by AICc is the one statsmodels chooses`, () => {
    assert.equal(result().model, ref.selected)
  })

  test(`${name}: point forecasts are within 1 % and the 95 % band within 3 % of statsmodels`, () => {
    const r = result()
    const want = (ref.models as Record<string, Ref>)[r.model]
    assert.equal(r.point.length, ref.horizon)
    r.point.forEach((v, i) => assert.ok(relative(v, want.mean[i]) < 0.01, `point ${i + 1}`))
    r.lower.forEach((v, i) => assert.ok(relative(v, want.lower[i]) < 0.03, `lower ${i + 1}`))
    r.upper.forEach((v, i) => assert.ok(relative(v, want.upper[i]) < 0.03, `upper ${i + 1}`))
  })
}

test("the band widens with the horizon and the chosen seasonal model is described in words", () => {
  const r = ok(forecast({ values: reference.airline.values.map(Math.log), grain: "month", horizon: 12 }))
  for (let i = 1; i < r.point.length; i++) {
    assert.ok(r.upper[i] - r.lower[i] >= r.upper[i - 1] - r.lower[i - 1])
  }
  assert.equal(r.seasonLength, 12)
  assert.equal(r.label, "level, trend and seasons")
})

test("a series with a line and no noise is forecast along the line, with no band", () => {
  const r = ok(forecast({ values: Array.from({ length: 20 }, (_, i) => 10 + 2 * i), grain: "year", horizon: 3 }))
  assert.equal(r.model, "AAN")
  r.point.forEach((v, i) => assert.ok(Math.abs(v - (10 + 2 * (20 + i))) < 1e-6))
  assert.ok(r.upper[2] - r.lower[2] < 1e-6)
})

test("four points are refused with the number of periods the chart has", () => {
  assert.equal(refusal(forecast({ values: [1, 2, 3, 4], grain: "month", horizon: 3 })), "A forecast needs at least 5 periods; this chart has 4.")
  assert.ok(!isRefusal(forecast({ values: [1, 2, 3, 4, 6], grain: "month", horizon: 3 })))
})

test("23 monthly points get no seasons and the note says why; 24 get them", () => {
  const month = (n: number) => Array.from({ length: n }, (_, i) => 100 + i + 10 * Math.sin((2 * Math.PI * i) / 12) + (i % 5) * 0.3)
  const short = ok(forecast({ values: month(23), grain: "month", horizon: 6 }))
  assert.equal(short.seasonLength, null)
  assert.ok(short.notes.some((n) => n.includes("fewer than 2 full seasons")))
  const enough = ok(forecast({ values: month(36), grain: "month", horizon: 6 }))
  assert.equal(enough.seasonLength, 12)
})

test("a year has no seasons, whatever the history", () => {
  assert.equal(seasonLength("year"), null)
  assert.equal(seasonLength("minute"), null)
  assert.deepEqual(["hour", "day", "week", "month", "quarter"].map(seasonLength), [24, 7, 52, 12, 4])
  const r = ok(forecast({ values: Array.from({ length: 60 }, (_, i) => i + (i % 2)), grain: "year", horizon: 2 }))
  assert.equal(r.seasonLength, null)
})

const gappy = (n: number, gaps: number) => {
  const v: (number | null)[] = Array.from({ length: n }, (_, i) => 50 + i + ((i * 7) % 5))
  for (let g = 0; g < gaps; g++) v[3 + g * 4] = null
  return v
}

test("12 % of periods missing is refused", () => {
  assert.match(refusal(forecast({ values: gappy(50, 6), grain: "year", horizon: 3 })), /6 of the 50 periods from the first to the last have no data, more than 10 %/)
})

test("8 % missing is filled in a straight line and counted", () => {
  const r = ok(forecast({ values: gappy(50, 4), grain: "year", horizon: 3 }))
  assert.equal(r.filled, 4)
  assert.ok(r.notes.some((n) => n.startsWith("4 periods without a value were filled")))
  const solo = ok(forecast({ values: [1, 2, null, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31], grain: "year", horizon: 2 }))
  assert.ok(solo.notes.some((n) => n.includes("1 period without a value was filled")))
})

test("empty periods at either end are trimmed, not refused (BI-17 B QA fix F1)", () => {
  const r = ok(forecast({ values: [null, null, 1, 2, 3, 4, 5, 6, 7, 8], grain: "year", horizon: 2 }))
  assert.equal(r.periodsUsed, 8)
  assert.equal(r.firstIndex, 2)
  const end = ok(forecast({ values: [1, 2, 3, 4, 5, 6, null, null], grain: "year", horizon: 2 }))
  assert.equal(end.periodsUsed, 6)
  assert.equal(end.lastIndex, 5)
  assert.ok(end.notes.includes("2 empty periods at the end are left out."))
  assert.match(refusal(forecast({ values: [null, null, null], grain: "year", horizon: 2 })), /at least 5 periods; this chart has 0/)
})

test("the unfinished current period is left out, by a fake clock", () => {
  const values = [10, 12, 13, 15, 16, 18, 19, 21, 22, 4]
  const end = Date.UTC(2026, 9, 31)
  const during = ok(forecast({ values, grain: "month", horizon: 2, lastPeriodEnd: end, now: Date.UTC(2026, 9, 24) }))
  assert.equal(during.droppedCurrent, true)
  assert.equal(during.periodsUsed, 9)
  assert.ok(during.notes[0].includes("not finished"))
  const after = ok(forecast({ values, grain: "month", horizon: 2, lastPeriodEnd: end, now: Date.UTC(2026, 10, 2) }))
  assert.equal(after.droppedCurrent, false)
  assert.equal(after.periodsUsed, 10)
  // The dropped period can bring the history under the minimum.
  const tiny = refusal(forecast({ values: [1, 2, 3, 4, 5], grain: "month", horizon: 2, lastPeriodEnd: end, now: Date.UTC(2026, 9, 24) }))
  assert.match(tiny, /this chart has 4 \(the unfinished current period is left out\)/)
})

test("the horizon is held to 60 and to the length of the history, and says so", () => {
  const r = ok(forecast({ values: Array.from({ length: 8 }, (_, i) => i * 3 + (i % 2)), grain: "year", horizon: 40 }))
  assert.equal(r.point.length, 8)
  assert.ok(r.notes.some((n) => n.includes("cut to 8 periods")))
  const long = ok(forecast({ values: Array.from({ length: 200 }, (_, i) => i + (i % 3)), grain: "year", horizon: 500 }))
  assert.equal(long.point.length, 60)
})

test("a constant series is forecast flat and does not break the fit", () => {
  const r = ok(forecast({ values: Array(12).fill(7), grain: "year", horizon: 3 }))
  assert.equal(r.model, "ANN")
  r.point.forEach((v) => assert.ok(Math.abs(v - 7) < 1e-9))
})
