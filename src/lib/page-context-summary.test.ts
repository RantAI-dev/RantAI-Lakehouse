import { strict as assert } from "node:assert"
import { test } from "node:test"
import { summarizeFilters, summarizeQuery, summarizeRows, summarizeTiles } from "./page-context-summary"

test("summarizeRows shows category=value pairs and counts the rest", () => {
  const rows = [1, 2, 3, 4, 5, 6, 7].map((i) => ({ region: `r${i}`, total: i * 10 }))
  assert.equal(
    summarizeRows(["region", "total"], rows, 3),
    "rows: r1=10, r2=20, r3=30 (+4 more)"
  )
  assert.equal(summarizeRows(["a"], []), "no rows")
})

test("summarizeRows spells out wider rows column by column", () => {
  assert.equal(
    summarizeRows(["a", "b", "c"], [{ a: 1, b: null, c: "x" }]),
    "rows: {a=1, b=∅, c=x}"
  )
})

test("summarizeTiles names the data source and reports errors and missing data honestly", () => {
  const text = summarizeTiles(
    [
      { id: "u_1", title: "Sales", kind: "bar", source: "ui", mart: "mart_sales" },
      { id: "u_2", title: "Join", kind: "table", source: "ai", sqlSource: "s_9" },
      { id: "k1", title: "Total", kind: "kpi", source: "builtin", mart: "mart_x" },
      { id: "t1", title: "Note", kind: "text", source: "ui" },
    ],
    {
      u_1: { columns: ["region", "v"], rows: [{ region: "north", v: 5 }] },
      u_2: { error: "the SQL source failed to run" },
    }
  )
  assert.equal(
    text,
    [
      '- "Sales" (bar, id u_1; mart_sales): rows: north=5',
      '- "Join" (table, id u_2; SQL source s_9): error: the SQL source failed to run',
      '- "Total" (kpi, built-in; mart_x): not loaded',
      '- "Note" (text, id t1; no data source): note',
    ].join("\n")
  )
})

test("summaries stay within their character budget", () => {
  const tiles = Array.from({ length: 50 }, (_, i) => ({ id: `u${i}`, title: `Tile ${i}`, kind: "bar", source: "ui", mart: "m" }))
  const out = summarizeTiles(tiles, {}, { maxChars: 300 })
  assert.ok(out.length <= 300)
  assert.ok(out.endsWith("(truncated)"))
})

test("long values are clipped so one cell cannot eat the budget", () => {
  const out = summarizeRows(["k", "v"], [{ k: "x".repeat(200), v: 1 }])
  assert.ok(out.length < 60, out)
})

test("summarizeFilters lists active filters and the year, or says none", () => {
  assert.equal(summarizeFilters([], "all"), "none")
  assert.equal(
    summarizeFilters([{ column: "region", values: ["north", "south"] }, { column: "x", values: [] }], "2024"),
    "region in [north, south]; year 2024"
  )
})

test("summarizeQuery includes the SQL and the last result, or says it has not run", () => {
  assert.equal(summarizeQuery("SELECT 1", null), "SQL in the editor:\nSELECT 1\nNot run yet.")
  const out = summarizeQuery("SELECT a, b FROM t", { columns: ["a", "b"], rows: [{ a: 1, b: 2 }], rowCount: 1 })
  assert.ok(out.includes("Last result: 1 row(s), columns a, b; rows: 1=2"), out)
})
