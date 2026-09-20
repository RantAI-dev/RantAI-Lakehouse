import { strict as assert } from "node:assert"
import { test } from "node:test"
import { parseDraft } from "./query-draft"

test("parseDraft mengembalikan null untuk isi yang tidak terpakai", () => {
  assert.equal(parseDraft(null), null)
  assert.equal(parseDraft("bukan json"), null)
  assert.equal(parseDraft(JSON.stringify({ sql: "   ", question: "" })), null)
})

test("parseDraft memulihkan draf yang berisi", () => {
  assert.deepEqual(
    parseDraft(JSON.stringify({ sql: "SELECT 1", question: "", tab: "sql" })),
    { sql: "SELECT 1", question: "", tab: "sql" }
  )
})

test("parseDraft memakai tab natural language bila nilainya tidak dikenal", () => {
  const draft = parseDraft(JSON.stringify({ sql: "", question: "berapa?", tab: "xyz" }))
  assert.equal(draft?.tab, "nl")
})
