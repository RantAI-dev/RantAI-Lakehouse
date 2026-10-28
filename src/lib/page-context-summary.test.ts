import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  summarizeFilterJson,
  summarizeFilters,
  summarizeGrain,
  summarizeParams,
  summarizeQuery,
  summarizeRows,
  summarizeTabs,
  summarizeTiles,
} from "./page-context-summary"

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

test("summarizeFilters describes each active filter as its chip reads, or says none", () => {
  assert.equal(summarizeFilters([]), "none")
  assert.equal(
    summarizeFilters([
      { column: "region", values: ["north", "south"] },
      { column: "x", values: [] },
      { column: "price", op: "between", values: [], min: "100" },
      { column: "day", op: "relative", values: [], anchor: "last", n: 30, unit: "day" },
    ]),
    "region is north, south; price ≥ 100; day in the last 30 days"
  )
})

test("summarizeGrain names the board's own grain, an override, or each chart's own", () => {
  // Board's own grain: shown equals saved, say the grain alone.
  assert.equal(summarizeGrain("day", "day"), "Time grain: Day")
  // Override: shown differs from saved, name the override and the saved default.
  assert.equal(summarizeGrain("month", "day"), "Time grain: Month (overrides saved Day)")
  // Each chart's own: the empty shown means the chart owns its grain, even with a saved default.
  assert.equal(summarizeGrain("", "day"), "Time grain: each chart's own")
  // No saved default with an explicit choice still names the choice.
  assert.equal(summarizeGrain("week", undefined), "Time grain: Week")
})

test("summarizeFilterJson returns the compact JSON when under budget, and null when over", () => {
  const filters = [
    { column: "province", values: ["Bali", "Aceh"] },
    { column: "price", op: "between" as const, values: [], min: "100" },
  ]
  const json = summarizeFilterJson(filters, { maxChars: 200 })
  assert.ok(json)
  const parsed = JSON.parse(json!)
  assert.equal(parsed.length, 2)
  // `normalizeFilters` orders by column, so `price` sorts before `province`.
  assert.equal(parsed[0].column, "price")
  assert.equal(parsed[1].column, "province")
  // Over the budget: the JSON is dropped, the chip text stays.
  const padded = Array.from({ length: 50 }, (_, i) => ({ column: `col_${i}`, values: ["x".repeat(40)] }))
  const drop = summarizeFilterJson(padded, { maxChars: 200 })
  assert.equal(drop, null)
})

test("summarizeQuery includes the SQL and the last result, or says it has not run", () => {
  assert.equal(summarizeQuery("SELECT 1", null), "SQL in the editor:\nSELECT 1\nNot run yet.")
  const out = summarizeQuery("SELECT a, b FROM t", { columns: ["a", "b"], rows: [{ a: 1, b: 2 }], rowCount: 1 })
  assert.ok(out.includes("Last result: 1 row(s), columns a, b; rows: 1=2"), out)
})

test("a tile with filters of its own says so, beside its numbers", () => {
  const out = summarizeTiles(
    [{ id: "u_1", title: "Visitors", kind: "bar", source: "ui", mart: "m", def: { filters: [{ column: "category", values: ["Office"] }] } }],
    { u_1: { columns: ["kab", "v"], rows: [{ kab: "Bali", v: 3 }] } },
  )
  assert.match(out, /\(bar, id u_1; m; filtered: category is Office\): rows: Bali=3/)
  const plain = summarizeTiles([{ id: "u_2", title: "T", kind: "bar", source: "ui", mart: "m" }], {})
  assert.doesNotMatch(plain, /filtered/)
})

test("a tile with bins says what its numbers stand for", () => {
  const out = summarizeTiles(
    [{ id: "c1", title: "Visits by size", kind: "bar", source: "ui", mart: "m", def: { dimension: "visitors", bin: { mode: "auto" } } }],
    { c1: { columns: ["visitors", "n"], rows: [{ visitors: 500, n: 4 }], bins: { visitors: { mode: "auto", width: 500, min: 44, max: 2377 } } } },
  )
  assert.match(out, /binned: visitors in bins of 500 \(a value is the bin's lower edge\)/)
})

test("BI-11: summarizeParams lists each shown parameter with its value and type, or says none", () => {
  assert.equal(summarizeParams([]), "none")
  assert.equal(
    summarizeParams([
      { param: { name: "target", type: "number" }, value: 1000 },
      { param: { name: "pick", type: "list" }, value: "rooms" },
    ]),
    "target = 1000 (number); pick = rooms (list)",
  )
})

test("a chart's other questions are listed beside its own rows, with their reason when they failed (BI-13)", () => {
  const tile = { id: "c_1", title: "Visitors and rooms", kind: "bar", source: "ui", mart: "mart_x" }
  const cell = {
    columns: ["month", "visitors"], rows: [{ month: "2026-01-01", visitors: 5 }],
    layers: [
      { label: "Rooms", draw: "line", axis: "right", source: "mart_rooms", columns: ["month", "rooms"], rows: [{ month: "2026-01-01", rooms: 9 }] },
      { label: "Occupancy", draw: "line", axis: "right", source: "mart_occ", columns: [], rows: [], error: "This chart could not be loaded." },
    ],
  }
  const text = summarizeTiles([tile], { c_1: cell })
  assert.match(text, /rows: 2026-01-01=5 \| also "Rooms" \(mart_rooms; line, right axis\): rows: 2026-01-01=9/)
  assert.match(text, /also "Occupancy" \(mart_occ; line, right axis\): error: This chart could not be loaded\./)
})

test("BI-18 part C: summarizeTabs names each tab with its id, the open one and the charts on it, or says none", () => {
  assert.equal(summarizeTabs([], () => undefined, ["a"]), "none (the dashboard has no tabs)")
  const tabs = [{ id: "t_1", title: "Overview" }, { id: "t_2", title: "Maps" }]
  const on = (id: string) => (id === "m" ? "t_2" : "t_1")
  assert.equal(
    summarizeTabs(tabs, on, ["a", "m", "b"], "t_2"),
    '"Overview" (id t_1; charts a, b); "Maps" (id t_2, open; charts m)',
  )
  assert.equal(summarizeTabs(tabs, () => "t_1", ["a"]), '"Overview" (id t_1; charts a); "Maps" (id t_2; no charts)')
})

test("summarizeTabs stays inside its budget: it counts the charts per tab when their ids do not fit, and keeps the ids when they do", () => {
  const tabs = [
    { id: "t_1", title: "Overview" },
    { id: "t_2", title: "Maps" },
    { id: "t_3", title: "Detail" },
  ]
  const on = (id: string) => (id === "m" || id === "n" ? "t_2" : "t_1")
  const ids = ["a", "b", "c", "m", "n"]
  const hint = "; list_charts gives each chart's tab"
  // The chart ids do not fit in 60 characters, so each tab carries a count.
  const capped = summarizeTabs(tabs, on, ids, "t_1", { maxChars: 60 })
  assert.match(capped, /3 charts/)
  assert.doesNotMatch(capped, /charts a, b, c/)
  assert.ok(capped.endsWith(hint), capped)
  assert.ok(capped.length <= 60 + hint.length, capped)
  // With room to spare the ids stay and no hint is added.
  const full = summarizeTabs(tabs, on, ids, "t_1", { maxChars: 600 })
  assert.equal(
    full,
    '"Overview" (id t_1, open; charts a, b, c); "Maps" (id t_2; charts m, n); "Detail" (id t_3; no charts)'
  )
  assert.doesNotMatch(full, /list_charts/)
})
