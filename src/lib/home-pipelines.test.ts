import { strict as assert } from "node:assert"
import { test } from "node:test"
import { formatUntil, pipelineRows } from "./home-pipelines"

const NOW = Date.parse("2026-10-02T06:00:00Z")

test("a pipeline that never ran and is not scheduled has nothing to report", () => {
  const rows = pipelineRows([
    { id: "a", name: "manual_job", status: "unknown", lastRunAt: null },
    { id: "b", name: "nightly", status: "completed", lastRunAt: "2026-10-02T03:00:00Z", nextRunAt: "2026-10-03T03:00:00Z" },
    { id: "c", name: "scheduled_never_ran", status: "unknown", lastRunAt: null, nextRunAt: "2026-10-02T07:00:00Z" },
  ])
  assert.deepEqual(rows.map((r) => r.id), ["b", "c"])
})

test("failed pipelines lead, then the most recently run", () => {
  const rows = pipelineRows([
    { id: "old", name: "old", status: "completed", lastRunAt: "2026-10-01T00:00:00Z" },
    { id: "new", name: "new", status: "completed", lastRunAt: "2026-10-02T05:00:00Z" },
    { id: "bad", name: "bad", status: "failed", lastRunAt: "2026-09-30T00:00:00Z" },
  ])
  assert.deepEqual(rows.map((r) => r.id), ["bad", "new", "old"])
})

test("the list is capped and an unparseable time is treated as absent", () => {
  const many = Array.from({ length: 9 }, (_, i) => ({
    id: `p${i}`, name: `p${i}`, status: "completed", lastRunAt: `2026-10-0${(i % 2) + 1}T0${i}:00:00Z`,
  }))
  assert.equal(pipelineRows(many).length, 5)
  assert.deepEqual(pipelineRows([{ id: "x", name: "x", status: "completed", lastRunAt: "not a date" }]), [])
})

test("formatUntil reads forward and refuses the past", () => {
  assert.equal(formatUntil("2026-10-02T06:00:20Z", NOW), "now")
  assert.equal(formatUntil("2026-10-02T06:45:00Z", NOW), "in 45m")
  assert.equal(formatUntil("2026-10-02T09:00:00Z", NOW), "in 3h")
  assert.equal(formatUntil("2026-10-04T06:00:00Z", NOW), "in 2d")
  assert.equal(formatUntil("2026-10-02T05:00:00Z", NOW), null)
  assert.equal(formatUntil(null, NOW), null)
})
