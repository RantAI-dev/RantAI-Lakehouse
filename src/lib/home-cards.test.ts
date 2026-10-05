import { strict as assert } from "node:assert"
import { test } from "node:test"
import { cardRead, openAlertRows, savedQueryHref, savedQueryRows, sourceRows } from "./home-cards"

test("sources list the unhealthy first, then degraded, unknown and healthy", () => {
  const rows = sourceRows([
    { id: "1", name: "b_ok", type: "postgres", health: "healthy" },
    { id: "2", name: "a_down", type: "s3", health: "unhealthy" },
    { id: "3", name: "c_slow", type: "kafka", health: "degraded" },
    { id: "4", name: "d_new", type: "mqtt", health: "unknown" },
    { id: "5", name: "a_ok", type: "postgres", health: "healthy" },
  ])
  assert.deepEqual(rows.map((r) => r.id), ["2", "3", "4", "5", "1"])
})

test("sources are capped and the input is left as it was", () => {
  const input = Array.from({ length: 8 }, (_, i) => ({
    id: String(i),
    name: `s${i}`,
    type: "postgres",
    health: "healthy",
  }))
  assert.equal(sourceRows(input, 3).length, 3)
  assert.equal(input[0].id, "0")
})

test("only open alerts are listed, most severe first, newest within a severity", () => {
  const rows = openAlertRows([
    { id: "a", title: "a", severity: "low", status: "open", at: "2026-10-04T09:00:00Z" },
    { id: "b", title: "b", severity: "critical", status: "open", at: "2026-10-04T07:00:00Z" },
    { id: "c", title: "c", severity: "critical", status: "open", at: "2026-10-04T08:00:00Z" },
    { id: "d", title: "d", severity: "critical", status: "resolved", at: "2026-10-04T10:00:00Z" },
    { id: "e", title: "e", severity: null, status: "open", at: "2026-10-04T11:00:00Z" },
    { id: "f", title: "f", severity: "acknowledged-ish", status: "acknowledged", at: "2026-10-04T11:00:00Z" },
  ])
  assert.deepEqual(rows.map((r) => r.id), ["c", "b", "a", "e"])
})

test("saved queries list the most recently changed first and link into Query Studio", () => {
  const rows = savedQueryRows([
    { id: "old", title: "old", updatedAt: "2026-09-01T00:00:00Z" },
    { id: "new", title: "new", updatedAt: "2026-10-03T00:00:00Z" },
    { id: "bad", title: "bad", updatedAt: "not a date" },
  ])
  assert.deepEqual(rows.map((r) => r.id), ["new", "old", "bad"])
  assert.equal(savedQueryHref("a b"), "/query-studio?saved=a%20b")
})

test("a failed read is told apart as refused (403, permission) or failed", () => {
  assert.equal(cardRead("loading", null), "loading")
  assert.equal(cardRead("success", null), "ok")
  assert.equal(cardRead("error", { code: "permission_denied" }), "denied")
  assert.equal(cardRead("error", { code: "invalid_request", status: 403 }), "denied")
  assert.equal(cardRead("error", { code: "unavailable", status: 500 }), "error")
  assert.equal(cardRead("error", null), "error")
})
