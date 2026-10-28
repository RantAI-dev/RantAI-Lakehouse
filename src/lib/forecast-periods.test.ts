import { strict as assert } from "node:assert"
import { test } from "node:test"
import { addPeriods, bucketEnd, futureLabels, layPeriods, parseBucket, wallClockMs } from "./forecast-periods"

// BI-17 part B (plan D4, section 5: how the date buckets arrive).
const b = (text: string) => {
  const parsed = parseBucket(text)
  assert.ok(parsed)
  return parsed
}

test("future labels continue the grain across the year end", () => {
  assert.deepEqual(futureLabels("month", b("2026-11-01"), 3), ["Dec 2026", "Jan 2027", "Feb 2027"])
  assert.deepEqual(futureLabels("quarter", b("2026-07-01"), 3), ["Q4 2026", "Q1 2027", "Q2 2027"])
  assert.deepEqual(futureLabels("year", b("2026-01-01"), 2), ["2027", "2028"])
  assert.deepEqual(futureLabels("week", b("2026-12-28"), 2), ["4 Jan 2027", "11 Jan 2027"])
  assert.deepEqual(futureLabels("day", b("2026-02-27"), 3), ["2026-02-28", "2026-03-01", "2026-03-02"])
  assert.deepEqual(futureLabels("hour", b("2026-03-09 22:00:00"), 3), ["9 Mar 23:00", "10 Mar 00:00", "10 Mar 01:00"])
})

test("a bucket that is not a real date is not read", () => {
  assert.equal(parseBucket("2026-02-31"), null)
  assert.equal(parseBucket("Mar 2026"), null)
  assert.equal(parseBucket(null), null)
})

test("a bucket ends where the next begins", () => {
  assert.equal(bucketEnd("month", b("2026-02-01")), Date.UTC(2026, 2, 1))
  assert.equal(bucketEnd("week", b("2026-10-19")), Date.UTC(2026, 9, 26))
  assert.equal(bucketEnd("quarter", b("2026-10-01")), Date.UTC(2027, 0, 1))
})

test("the wall clock in a zone is not the UTC clock", () => {
  const instant = Date.UTC(2026, 9, 31, 20, 30) // 20:30 UTC on 31 Oct
  assert.equal(wallClockMs(instant, "Asia/Jakarta"), Date.UTC(2026, 10, 1, 3, 30)) // UTC+7: already 1 Nov
  assert.equal(wallClockMs(instant, "America/Los_Angeles"), Date.UTC(2026, 9, 31, 13, 30)) // UTC-7 in October
  assert.equal(wallClockMs(instant, "Not/AZone"), instant)
})

test("rows are laid on a regular grid, a month with no row is a gap", () => {
  const laid = layPeriods("month", ["2026-01-01", "2026-02-01", "2026-04-01"], [1, 2, 4])
  assert.deepEqual(laid?.starts, ["2026-01-01", "2026-02-01", "2026-03-01", "2026-04-01"])
  assert.deepEqual(laid?.values, [1, 2, null, 4])
  assert.deepEqual(laid?.last, b("2026-04-01"))
})

test("buckets that are not one run of dates are refused, not forecast", () => {
  assert.equal(layPeriods("month", ["2026-02-01", "2026-01-01"], [1, 2]), null)
  assert.equal(layPeriods("month", ["2026-01-01", "2026-01-01"], [1, 2]), null)
  assert.equal(layPeriods("month", ["2026-01-05", "2026-02-05"], [1, 2]), null)
  assert.equal(layPeriods("month", ["north", "south"], [1, 2]), null)
  assert.equal(layPeriods("month_of_year", ["1", "2"], [1, 2]), null)
  assert.equal(layPeriods("minute", ["2000-01-01 00:00:00", "2026-01-01 00:00:00"], [1, 2]), null)
})

test("addPeriods steps back as well as forward", () => {
  assert.deepEqual(addPeriods("month", b("2026-01-01"), -1), b("2025-12-01"))
})
