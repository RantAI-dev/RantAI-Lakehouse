import assert from "node:assert/strict"
import test from "node:test"

import type { TableDefFields } from "./table-types"
import { cleanSetting, cleanSettings, draftFromDef, draftPayload, draftProblems, emptyTableDraft, moveItem } from "./table-draft"

test("a saved raw table opens as the draft it was made from and goes back unchanged", () => {
  const def: TableDefFields = {
    tableMode: "rows", columns: ["place", "visitors"], sortColumn: "visitors", sortDir: "desc",
    columnSettings: { visitors: { format: "currency", decimals: 0, width: 120 } },
  }
  const d = draftFromDef(def)
  assert.equal(d.mode, "rows")
  assert.deepEqual(d.columns, ["place", "visitors"])
  assert.deepEqual(draftPayload("table", d), {
    tableMode: "rows", columns: ["place", "visitors"], sortColumn: "visitors", sortDir: "desc",
    columnSettings: { visitors: { format: "currency", decimals: 0, width: 120 } },
  })
})

test("a hidden column, a swapped order and a currency format survive the round trip", () => {
  let d = draftFromDef({ tableMode: "rows", columns: ["a", "b", "c"] })
  d = { ...d, columns: moveItem(d.columns, 0, 1), settings: { c: { hidden: true }, b: { format: "currency" } } }
  const payload = draftPayload("table", d) as { columns: string[]; columnSettings: Record<string, unknown> }
  assert.deepEqual(payload.columns, ["b", "a", "c"])
  assert.deepEqual(payload.columnSettings, { b: { format: "currency" }, c: { hidden: true } })
  const again = draftFromDef({ tableMode: "rows", ...payload } as never)
  assert.deepEqual(again.columns, ["b", "a", "c"])
  assert.equal(again.settings.c.hidden, true)
})

test("settings that say nothing, or belong to a column no longer shown, are not sent", () => {
  assert.equal(cleanSetting({ format: "auto", wrap: false, hidden: false, label: "  " }), null)
  assert.deepEqual(cleanSetting({ label: " Visitors ", decimals: 0 }), { label: "Visitors", decimals: 0 })
  assert.equal(cleanSettings({ gone: { hidden: true } }, ["a"]), undefined)
  assert.deepEqual(cleanSettings({ a: { wrap: true }, gone: { hidden: true } }, ["a"]), { a: { wrap: true } })
})

test("a grouped table sends only its column settings, for the columns it shows", () => {
  const d = { ...emptyTableDraft(), settings: { place: { label: "Where" }, other: { label: "x" } } }
  assert.deepEqual(draftPayload("table", d, ["place", "visitors"]), { columnSettings: { place: { label: "Where" } } })
  assert.deepEqual(draftPayload("bar", d), {})
})

test("a pivot sends rows, optional columns, the values that are picked and its totals", () => {
  const d = {
    ...emptyTableDraft(), pivotRows: ["province", "kind"], pivotColumns: [], totals: "all" as const,
    pivotValues: [{ column: "visitors", aggregate: "sum" }, { column: "", aggregate: "sum" }],
  }
  assert.deepEqual(draftPayload("pivot", d), {
    rows: ["province", "kind"], columns: undefined, values: [{ column: "visitors", aggregate: "sum" }], totals: "all", columnSettings: undefined,
  })
  assert.deepEqual(draftProblems("pivot", emptyTableDraft()), ["Rows", "Values"])
  assert.deepEqual(draftProblems("pivot", d), [])
})

test("a pivot saved with columns opens with them, and a raw table's columns are not a pivot's", () => {
  const p = draftFromDef({ rows: ["a"], columns: ["m"], values: [{ column: "v", aggregate: "avg" }], totals: "grand" })
  assert.deepEqual(p.pivotColumns, ["m"])
  assert.deepEqual(p.columns, [])
  assert.equal(p.totals, "grand")
  const t = draftFromDef({ tableMode: "rows", columns: ["x"] })
  assert.deepEqual(t.pivotColumns, [])
})

test("a KPI sends a comparison only when it is complete", () => {
  const base = emptyTableDraft()
  assert.deepEqual(draftPayload("kpi", base), { compare: undefined, goodDirection: undefined })
  const prev = { ...base, compareKind: "previous" as const, compareColumn: "day", comparePeriod: "week" as const, goodDirection: "down" as const }
  assert.deepEqual(draftPayload("kpi", prev), { compare: { kind: "previous", dateColumn: "day", period: "week" }, goodDirection: "down" })
  assert.deepEqual(draftProblems("kpi", { ...prev, compareColumn: "" }), ["Date column"])
  const goal = { ...base, compareKind: "goal" as const, goal: "2500" }
  assert.deepEqual(draftPayload("kpi", goal), { compare: { kind: "goal", value: 2500 }, goodDirection: undefined })
  assert.deepEqual(draftProblems("kpi", { ...goal, goal: "" }), ["Goal"])
  assert.deepEqual(draftProblems("kpi", { ...goal, goal: "abc" }), ["Goal"])
  const reopened = draftFromDef({ compare: { kind: "goal", value: 2500 }, goodDirection: "down" })
  assert.equal(reopened.compareKind, "goal")
  assert.equal(reopened.goal, "2500")
  assert.equal(reopened.goodDirection, "down")
})

test("moving an item stays inside the list", () => {
  assert.deepEqual(moveItem(["a", "b", "c"], 2, 1), ["a", "b", "c"])
  assert.deepEqual(moveItem(["a", "b", "c"], 0, -1), ["a", "b", "c"])
  assert.deepEqual(moveItem(["a", "b", "c"], 2, -1), ["a", "c", "b"])
})

test("a raw table needs at least one column", () => {
  assert.deepEqual(draftProblems("table", { ...emptyTableDraft(), mode: "rows" }), ["Columns"])
  assert.deepEqual(draftProblems("table", emptyTableDraft()), [])
})
