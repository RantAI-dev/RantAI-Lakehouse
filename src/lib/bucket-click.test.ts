import assert from "node:assert/strict"
import test from "node:test"

import { decodeFilters } from "./dashboard-filter-state"
import {
  bucketActions,
  bucketRangeFilter,
  dashboardRangeDestination,
  toggleBucketFilter,
} from "./bucket-click"

test("a day and coarser can filter and list; minute and hour list only; a part neither", () => {
  assert.deepEqual(bucketActions("month"), { filter: true, records: true })
  assert.deepEqual(bucketActions("day"), { filter: true, records: true })
  assert.deepEqual(bucketActions("hour"), { filter: false, records: true })
  assert.deepEqual(bucketActions("minute"), { filter: false, records: true })
  assert.deepEqual(bucketActions("day_of_week"), { filter: false, records: false })
  assert.deepEqual(bucketActions("hour_of_day"), { filter: false, records: false })
})

test("a month becomes one between filter from its first to its last day", () => {
  assert.deepEqual(bucketRangeFilter("visit_date", "month", "2026-03-01"), {
    column: "visit_date", values: [], op: "between", min: "2026-03-01", max: "2026-03-31",
  })
  assert.equal(bucketRangeFilter("visit_date", "month", "2028-02-01")?.max, "2028-02-29")
  assert.equal(bucketRangeFilter("visit_date", "quarter", "2026-04-01")?.max, "2026-06-30")
})

test("a week's range follows its first day, a Monday or a Sunday", () => {
  assert.equal(bucketRangeFilter("d", "week", "2026-03-09")?.max, "2026-03-15")
  assert.equal(bucketRangeFilter("d", "week", "2026-03-08")?.max, "2026-03-14")
})

test("an hour, a part and an empty bucket have no range filter", () => {
  assert.equal(bucketRangeFilter("d", "hour", "2026-03-09 10:00:00"), null)
  assert.equal(bucketRangeFilter("d", "day_of_week", "1"), null)
  assert.equal(bucketRangeFilter("d", "month", ""), null)
})

test("a second click on the same bucket clears it, another bucket replaces it, other columns stay", () => {
  const march = bucketRangeFilter("d", "month", "2026-03-01")!
  const april = bucketRangeFilter("d", "month", "2026-04-01")!
  const other = { column: "provinsi", values: ["Bali"] }
  const once = toggleBucketFilter([other], march)
  assert.deepEqual(once, [other, march])
  assert.deepEqual(toggleBucketFilter(once, april), [other, april])
  assert.deepEqual(toggleBucketFilter(once, march), [other])
})

test("a dashboard destination carries the range in the address the page reads", () => {
  const filter = bucketRangeFilter("d", "month", "2026-03-01")!
  const href = dashboardRangeDestination("b 1", filter)
  assert.ok(href.startsWith("/dashboards/b%201?f="), href)
  const f = new URL(href, "http://x").searchParams.get("f")
  assert.deepEqual(decodeFilters(f), [filter])
})
