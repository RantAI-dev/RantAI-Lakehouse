import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  NEW_SQL_CHOICE,
  fieldsFromColumns,
  formatCell,
  isNumericColumnType,
  keepIfOffered,
  otherChartsUsing,
  saveBlocker,
  sourceNameFor,
  sourceNameProblem,
  sqlChanged,
  sqlRunPhase,
} from "./sql-source-draft"
import { decodeSourceChoice } from "./chart-source"

test("the new-SQL select value is never read as a mart or a saved source", () => {
  assert.equal(decodeSourceChoice(NEW_SQL_CHOICE), null)
})

test("numeric column types are measures, including nullable and decimal ones", () => {
  for (const t of ["UInt16", "Int64", "Float64", "Decimal(10, 2)", "Nullable(Int32)"]) {
    assert.equal(isNumericColumnType(t), true, t)
  }
  for (const t of ["String", "Date", "Nullable(String)", "DateTime64(3)"]) {
    assert.equal(isNumericColumnType(t), false, t)
  }
})

test("preview columns are split into dimensions and measures in column order", () => {
  const fields = fieldsFromColumns([
    { name: "region", type: "String" },
    { name: "visits", type: "UInt64" },
    { name: "day", type: "Date" },
    { name: "lat", type: "Nullable(Float64)" },
  ])
  assert.deepEqual(fields, { dimensions: ["region", "day"], measures: ["visits", "lat"] })
})

test("a picked column the re-run no longer returns is cleared, one it still returns is kept", () => {
  assert.equal(keepIfOffered("visits", ["visits", "lat"]), "visits")
  assert.equal(keepIfOffered("gone", ["visits", "lat"]), "")
  assert.equal(keepIfOffered("", ["visits"]), "")
})

test("a successful run is fresh only while the SQL is unchanged", () => {
  const ran = "SELECT 1 FROM serving.t"
  assert.equal(sqlRunPhase({ act: "success", ranSql: ran, sql: ran }), "fresh")
  assert.equal(sqlRunPhase({ act: "success", ranSql: ran, sql: `${ran}  \n` }), "fresh")
  assert.equal(sqlRunPhase({ act: "success", ranSql: ran, sql: "SELECT 2 FROM serving.t" }), "stale")
})

test("a failed run describes only the text that failed", () => {
  const ran = "SELECT x FROM serving.t"
  assert.equal(sqlRunPhase({ act: "error", ranSql: ran, sql: ran }), "failed")
  assert.equal(sqlRunPhase({ act: "error", ranSql: ran, sql: "SELECT y FROM serving.t" }), "unrun")
})

test("empty, comment-only, never-run and in-flight SQL have their own phases", () => {
  assert.equal(sqlRunPhase({ act: "idle", ranSql: null, sql: "  " }), "empty")
  assert.equal(sqlRunPhase({ act: "success", ranSql: "x", sql: "-- nothing here" }), "empty")
  assert.equal(sqlRunPhase({ act: "idle", ranSql: null, sql: "SELECT 1" }), "unrun")
  assert.equal(sqlRunPhase({ act: "pending", ranSql: "SELECT 1", sql: "SELECT 1" }), "running")
})

test("the name follows the fallback until typed, and a cleared name stays empty", () => {
  assert.equal(sourceNameFor(null, "Visitors by region"), "Visitors by region")
  assert.equal(sourceNameFor("My source", "Visitors by region"), "My source")
  assert.equal(sourceNameFor("", "Visitors by region"), "")
})

test("a source name is required and limited to 200 characters", () => {
  assert.equal(sourceNameProblem("   "), "Source name is required.")
  assert.equal(sourceNameProblem("a".repeat(200)), null)
  assert.match(sourceNameProblem("a".repeat(201)) ?? "", /200 characters/)
  assert.equal(sourceNameProblem("  ok  "), null)
})

test("other charts are counted once each and never include the chart being edited", () => {
  const charts = [
    { id: "c1", def: { sqlSource: "s_1" } },
    { id: "c2", def: { sqlSource: "s_1" } },
    { id: "c2", def: { sqlSource: "s_1" } },
    { id: "c3", sqlSource: "s_1" },
    { id: "c4", def: { sqlSource: "s_2" } },
    { id: "c5", def: {} },
    { id: "c6" },
  ]
  assert.equal(otherChartsUsing(charts, "s_1"), 3)
  assert.equal(otherChartsUsing(charts, "s_1", "c1"), 2)
  assert.equal(otherChartsUsing(charts, "s_2", "c4"), 0)
  assert.equal(otherChartsUsing([], "s_1"), 0)
})

test("editing is detected on trimmed text", () => {
  assert.equal(sqlChanged("SELECT 1 ", "SELECT 1"), false)
  assert.equal(sqlChanged("SELECT 2", "SELECT 1"), true)
})

const base = { originalSql: "SELECT 1 FROM serving.t", name: "A source" }

test("a new source cannot be saved before its SQL ran", () => {
  assert.equal(saveBlocker({ ...base, mode: "new", sql: "SELECT 1", phase: "unrun" }), "Run the SQL before saving.")
  assert.equal(saveBlocker({ ...base, mode: "new", sql: "", phase: "empty" }), "Write the SQL first.")
  assert.equal(saveBlocker({ ...base, mode: "new", sql: "SELECT 1", phase: "running" }), "Wait for the SQL run to finish.")
})

test("SQL edited after a run asks for another run instead of saving", () => {
  assert.match(saveBlocker({ ...base, mode: "new", sql: "SELECT 2", phase: "stale" }) ?? "", /Run the SQL again/)
  assert.match(saveBlocker({ ...base, mode: "edit", sql: "SELECT 2", phase: "stale" }) ?? "", /Run the SQL again/)
  assert.match(saveBlocker({ ...base, mode: "new", sql: "SELECT 2", phase: "failed" }) ?? "", /did not run/)
})

test("a fresh run with a valid name can be saved", () => {
  assert.equal(saveBlocker({ ...base, mode: "new", sql: "SELECT 1", phase: "fresh" }), null)
})

test("a fresh run still cannot be saved without a name", () => {
  assert.equal(saveBlocker({ ...base, name: " ", mode: "new", sql: "SELECT 1", phase: "fresh" }), "Source name is required.")
})

test("an edit that leaves the SQL as it was needs no new run, only a name", () => {
  assert.equal(saveBlocker({ ...base, mode: "edit", sql: base.originalSql, phase: "unrun" }), null)
  assert.equal(
    saveBlocker({ ...base, name: "", mode: "edit", sql: base.originalSql, phase: "unrun" }),
    "Source name is required."
  )
})

test("preview cells keep NULL visible and objects readable", () => {
  assert.equal(formatCell(null), "NULL")
  assert.equal(formatCell(undefined), "NULL")
  assert.equal(formatCell(0), "0")
  assert.equal(formatCell(""), "")
  assert.equal(formatCell({ a: 1 }), '{"a":1}')
})
