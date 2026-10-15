import assert from "node:assert/strict"
import test from "node:test"

import { buildPivot, canCollapse, collapsibleGroups, columnLabel, groupId, rowLabel, visibleRows } from "./pivot"

// rows: province, kind; columns: month; one value. Flags: __g0 province, __g1 kind, __g2 month.
type R = Record<string, unknown>
const row = (province: unknown, kind: unknown, month: unknown, v: number | string, g: [number, number, number]): R => ({
  province, kind, month, __v0: v, __g0: g[0], __g1: g[1], __g2: g[2],
})
const input = (rows: R[]) => ({ rows, rowFields: ["province", "kind"], colFields: ["month"], valueCount: 1 })

const ALL: R[] = [
  row("Bali", "Hotel", "2026-01", 1, [0, 0, 0]),
  row("Bali", "Villa", "2026-01", 2, [0, 0, 0]),
  row("Bali", "Hotel", "2026-02", 4, [0, 0, 0]),
  row("Jabar", "Hotel", "2026-01", 8, [0, 0, 0]),
  // subtotals per province (kind rolled up)
  row("Bali", "", "2026-01", 3, [0, 1, 0]),
  row("Bali", "", "2026-02", 4, [0, 1, 0]),
  row("Jabar", "", "2026-01", 8, [0, 1, 0]),
  // row totals across months (month rolled up)
  row("Bali", "Hotel", "1970-01-01", 5, [0, 0, 1]),
  row("Bali", "Villa", "1970-01-01", 2, [0, 0, 1]),
  row("Jabar", "Hotel", "1970-01-01", 8, [0, 0, 1]),
  row("Bali", "", "1970-01-01", 7, [0, 1, 1]),
  row("Jabar", "", "1970-01-01", 8, [0, 1, 1]),
  // grand totals
  row("", "", "2026-01", 11, [1, 1, 0]),
  row("", "", "2026-02", 4, [1, 1, 0]),
  row("", "", "1970-01-01", 15, [1, 1, 1]),
]

test("rows are grouped, each group followed by its own subtotal and the grand total last", () => {
  const m = buildPivot(input(ALL))
  assert.deepEqual(m.rows.map((r) => `${r.kind}:${rowLabel(r)}`), [
    "leaf:Bali · Hotel", "leaf:Bali · Villa", "subtotal:Bali total",
    "leaf:Jabar · Hotel", "subtotal:Jabar total", "grand:Total",
  ])
  assert.deepEqual(m.columns.map(columnLabel), ["2026-01", "2026-02", "Total"])
})

test("every cell is the database's number: totals are placed, never summed", () => {
  const m = buildPivot(input(ALL))
  const byLabel = Object.fromEntries(m.rows.map((r) => [rowLabel(r), r.cells.map((c) => c[0])]))
  assert.deepEqual(byLabel["Bali · Hotel"], [1, 4, 5])
  assert.deepEqual(byLabel["Bali · Villa"], [2, null, 2])
  assert.deepEqual(byLabel["Bali total"], [3, 4, 7])
  assert.deepEqual(byLabel["Total"], [11, 4, 15])
})

test("a real empty or null group is not a total: only the flags say so", () => {
  const rows = [
    row("", "", "2026-01", 5, [0, 0, 0]), // province "" and kind "" are real values
    row(null, "Hotel", "2026-01", 6, [0, 0, 0]),
  ]
  const m = buildPivot(input(rows))
  assert.deepEqual(m.rows.map((r) => r.kind), ["leaf", "leaf"])
  assert.deepEqual(m.rows.map((r) => r.labels), [["(empty)", "(empty)"], ["(empty)", "Hotel"]])
  assert.equal(m.rows.length, 2)
})

test("a value that arrives as text is read as a number, and a missing cell stays empty", () => {
  const m = buildPivot(input([row("Bali", "Hotel", "2026-01", "12", [0, 0, 0]), row("Jabar", "Hotel", "2026-02", 1, [0, 0, 0])]))
  assert.equal(m.rows[0].cells[0][0], 12)
  assert.equal(m.rows[0].cells[1][0], null)
})

test("without totals there are no total rows or columns, and without column fields one column", () => {
  const body = ALL.filter((r) => r.__g0 === 0 && r.__g1 === 0 && r.__g2 === 0)
  const m = buildPivot(input(body))
  assert.deepEqual(m.rows.map((r) => r.kind), ["leaf", "leaf", "leaf"])
  assert.deepEqual(m.columns.map((c) => c.kind), ["leaf", "leaf"])
  const flat = buildPivot({
    rows: [{ province: "Bali", __v0: 3, __g0: 0 }, { province: "", __v0: 9, __g0: 1 }],
    rowFields: ["province"], colFields: [], valueCount: 1,
  })
  assert.equal(flat.columns.length, 1)
  assert.deepEqual(flat.rows.map((r) => `${r.kind}:${rowLabel(r)}`), ["leaf:Bali", "grand:Total"])
  assert.deepEqual(flat.rows[1].cells, [[9]])
})

test("several values give several cells per column", () => {
  const m = buildPivot({
    rows: [{ province: "Bali", __v0: 1, __v1: 2, __g0: 0 }],
    rowFields: ["province"], colFields: [], valueCount: 2,
  })
  assert.deepEqual(m.rows[0].cells, [[1, 2]])
})

test("folding a group keeps its subtotal and hides what is inside; only totals 'all' can fold", () => {
  const m = buildPivot(input(ALL))
  assert.deepEqual([...collapsibleGroups(m)], [groupId(["Bali"]), groupId(["Jabar"])])
  const folded = visibleRows(m, new Set([groupId(["Bali"])]))
  assert.deepEqual(folded.map(rowLabel), ["Bali total", "Jabar · Hotel", "Jabar total", "Total"])
  assert.equal(visibleRows(m, new Set()).length, m.rows.length)
  assert.equal(canCollapse("all"), true)
  assert.equal(canCollapse("grand"), false)
  assert.equal(canCollapse(undefined), false)
})

test("keys sort as numbers when they are numbers, and a null group goes last", () => {
  const m = buildPivot({
    rows: [10, 9, null, 2].map((p) => ({ p, __v0: 1, __g0: 0 })),
    rowFields: ["p"], colFields: [], valueCount: 1,
  })
  assert.deepEqual(m.rows.map((r) => r.labels[0]), ["2", "9", "10", "(empty)"])
})

test("a row whose flags are not a prefix shape is left out, not misplaced", () => {
  const odd = { province: "Bali", kind: "Hotel", month: "x", __v0: 1, __g0: 1, __g1: 0, __g2: 0 }
  assert.equal(buildPivot(input([odd])).rows.length, 0)
})
