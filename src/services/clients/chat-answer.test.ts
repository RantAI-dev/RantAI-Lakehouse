import { strict as assert } from "node:assert"
import { test } from "node:test"
import { chatAnswerFromBody } from "./chat-answer"

const OK_SQL = {
  tool: "run_sql",
  args: { sql: "SELECT region, count() AS n FROM mart.sales GROUP BY region" },
  ok: true,
  result: {
    columns: [
      { name: "region", type: "String" },
      { name: "n", type: "UInt64" },
    ],
    rows: [
      { region: "EU", n: 3 },
      { region: "US", n: 5 },
    ],
    rowCount: 2,
  },
}

test("one successful run_sql gives its SQL, column names, rows and row count", () => {
  const out = chatAnswerFromBody({ answer: "Two regions.", toolTrace: [OK_SQL] })
  assert.equal(out.answer, "Two regions.")
  assert.equal(out.sql, "SELECT region, count() AS n FROM mart.sales GROUP BY region")
  assert.deepEqual(out.columns, ["region", "n"])
  assert.deepEqual(out.rows, [
    { region: "EU", n: 3 },
    { region: "US", n: 5 },
  ])
  assert.equal(out.rowCount, 2)
})

test("a failed run_sql followed by a good one reports the second and keeps both steps", () => {
  const failed = {
    tool: "run_sql",
    args: { sql: "SELECT nope FROM mart.sales" },
    ok: false,
    result: { error: "Unknown column nope", hint: "Check the DATA MAP." },
  }
  const out = chatAnswerFromBody({ answer: "Done.", toolTrace: [failed, OK_SQL] })
  assert.equal(out.sql, OK_SQL.args.sql)
  assert.equal(out.steps.length, 2)
  assert.deepEqual(out.steps[0], { step: "run_sql", detail: "Unknown column nope" })
  assert.deepEqual(out.steps[1], { step: "run_sql", detail: OK_SQL.args.sql })
})

test("a trace with only list_datasets gives no SQL and no rows but keeps the answer", () => {
  const out = chatAnswerFromBody({
    answer: "These tables exist.",
    toolTrace: [{ tool: "list_datasets", args: {}, ok: true, result: { datasets: [] } }],
  })
  assert.equal(out.answer, "These tables exist.")
  assert.equal(out.sql, undefined)
  assert.deepEqual(out.columns, [])
  assert.deepEqual(out.rows, [])
  assert.equal(out.rowCount, 0)
  assert.deepEqual(out.steps, [{ step: "list_datasets", detail: "" }])
})

test("an empty trace gives no SQL and no rows but keeps the answer", () => {
  const out = chatAnswerFromBody({ answer: "I cannot tell.", toolTrace: [] })
  assert.equal(out.answer, "I cannot tell.")
  assert.equal(out.sql, undefined)
  assert.deepEqual(out.rows, [])
  assert.deepEqual(out.steps, [])
})

test("two successful run_sql calls report the last one's SQL", () => {
  const first = { ...OK_SQL, args: { sql: "SELECT 1" } }
  const out = chatAnswerFromBody({ answer: "Done.", toolTrace: [first, OK_SQL] })
  assert.equal(out.sql, OK_SQL.args.sql)
  assert.equal(out.steps.length, 2)
})
