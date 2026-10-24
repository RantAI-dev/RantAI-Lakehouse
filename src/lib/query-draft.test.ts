import { strict as assert } from "node:assert"
import { test } from "node:test"
import { parseDraft, parseTabs } from "./query-draft"

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

test("parseTabs memulihkan tab yang tersimpan dalam bentuk baru", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [
        { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" },
        { id: "b", name: "Users", sql: "SELECT 2", engine: "trino" },
      ],
      activeId: "b",
      question: "berapa?",
      tab: "sql",
    })
  )
  assert.deepEqual(tabs, {
    tabs: [
      { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" },
      { id: "b", name: "Users", sql: "SELECT 2", engine: "trino" },
    ],
    activeId: "b",
    question: "berapa?",
    tab: "sql",
  })
})

test("parseTabs membuang entri yang tidak bisa dipakai", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [
        { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" },
        { id: "", name: "Tanpa id", sql: "SELECT 2", engine: "clickhouse" },
        { id: "c", name: "Tanpa sql", engine: "clickhouse" },
        { id: "d", name: "Mesin lain", sql: "SELECT 3", engine: "postgres" },
        { id: "e", sql: "SELECT 4", engine: "clickhouse" },
        "bukan tab",
      ],
      activeId: "a",
    })
  )
  assert.deepEqual(tabs?.tabs, [
    { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" },
  ])
  assert.equal(tabs?.activeId, "a")
})

test("parseTabs membuang id yang kembar dan menyimpan yang pertama", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [
        { id: "a", name: "Pertama", sql: "SELECT 1", engine: "clickhouse" },
        { id: "a", name: "Kedua", sql: "SELECT 2", engine: "clickhouse" },
      ],
      activeId: "a",
    })
  )
  assert.equal(tabs?.tabs.length, 1)
  assert.equal(tabs?.tabs[0].name, "Pertama")
})

test("parseTabs memotong tab lebih dari 20 menjadi 20", () => {
  const stored = {
    tabs: Array.from({ length: 25 }, (_, index) => ({
      id: `t${index}`,
      name: `Query ${index}`,
      sql: `SELECT ${index}`,
      engine: "clickhouse",
    })),
    activeId: "t0",
  }
  const tabs = parseTabs(JSON.stringify(stored))
  assert.equal(tabs?.tabs.length, 20)
  assert.equal(tabs?.tabs[0].id, "t0")
  assert.equal(tabs?.tabs[19].id, "t19")
})

test("parseTabs memakai tab pertama ketika activeId tidak ada di dalam daftar", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [{ id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" }],
      activeId: "hilang",
    })
  )
  assert.equal(tabs?.activeId, "a")
})

test("parseTabs memindahkan draf lama menjadi satu tab", () => {
  const tabs = parseTabs(JSON.stringify({ sql: "SELECT 1", question: "berapa?", tab: "sql" }))
  assert.deepEqual(tabs, {
    tabs: [{ id: "tab-1", name: "Query 1", sql: "SELECT 1", engine: "clickhouse" }],
    activeId: "tab-1",
    question: "berapa?",
    tab: "sql",
  })
})

test("parseTabs mengembalikan null untuk isi yang bukan salah satu bentuk", () => {
  assert.equal(parseTabs(null), null)
  assert.equal(parseTabs("bukan json"), null)
  assert.equal(parseTabs(JSON.stringify({ sql: "   ", question: "" })), null)
  assert.equal(parseTabs(JSON.stringify({})), null)
  assert.equal(parseTabs(JSON.stringify({ tabs: [] })), null)
})
