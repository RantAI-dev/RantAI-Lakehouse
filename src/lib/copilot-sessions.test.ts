import { strict as assert } from "node:assert"
import { test } from "node:test"
import { groupSessions, plainPreview, sessionGroup } from "./copilot-sessions"

test("plainPreview membuang markdown dan memotong dengan elipsis", () => {
  const md = "I have prepared:\n\n* **Chart Type**: Bar (`bar`)\n* [Docs](https://x)\n```sql\nSELECT 1\n```"
  assert.equal(plainPreview(md), "I have prepared: Chart Type: Bar (bar) Docs")
  const cut = plainPreview("a ".repeat(200), 20)
  assert.equal(cut.length, 20)
  assert.ok(cut.endsWith("…"))
})

test("plainPreview keeps underscores inside identifiers while still dropping emphasis", () => {
  assert.equal(plainPreview("Run `bronze_ingest_job` on _mart_wisman_ and serving.mart_event"), "Run bronze_ingest_job on mart_wisman and serving.mart_event")
  assert.equal(plainPreview("**bold** and *it* and ~~gone~~"), "bold and it and gone")
})

test("plainPreview keeps an unverified number but drops its citation-marker span", () => {
  const md = 'I need two things: <span data-unverified="true">1</span>. What to ingest'
  assert.equal(plainPreview(md), "I need two things: 1. What to ingest")
})

test("sessionGroup mengelompokkan per hari kalender lokal", () => {
  const now = new Date(2026, 8, 19, 9, 0)
  const at = (d: number, h = 12) => new Date(2026, 8, d, h).toISOString()
  assert.equal(sessionGroup(at(19, 1), now), "Today")
  assert.equal(sessionGroup(at(18, 23), now), "Yesterday")
  assert.equal(sessionGroup(at(14), now), "Previous 7 days")
  assert.equal(sessionGroup(at(1), now), "Previous 30 days")
  assert.equal(sessionGroup(new Date(2026, 5, 1).toISOString(), now), "Earlier")
  assert.equal(sessionGroup(undefined, now), "Earlier")
})

test("groupSessions menjaga urutan kelompok dan membuang yang kosong", () => {
  const now = new Date(2026, 8, 19, 9, 0)
  const items = [
    { id: "old", updatedAt: new Date(2026, 5, 1).toISOString() },
    { id: "today", updatedAt: new Date(2026, 8, 19, 8).toISOString() },
  ]
  assert.deepEqual(
    groupSessions(items, now).map((g) => [g.group, g.items.map((i) => i.id)]),
    [["Today", ["today"]], ["Earlier", ["old"]]]
  )
})
