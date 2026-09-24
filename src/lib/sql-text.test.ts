import { strict as assert } from "node:assert"
import { test } from "node:test"
import { hasStatement, sqlSummary, stripSqlComments } from "./sql-text"

test("stripSqlComments membuang komentar baris dan blok", () => {
  assert.equal(stripSqlComments("-- catatan\nSELECT 1").trim(), "SELECT 1")
  assert.equal(stripSqlComments("SELECT 1 # ekor").trim(), "SELECT 1")
  assert.equal(stripSqlComments("/* blok */SELECT 1").trim(), "SELECT 1")
  assert.equal(stripSqlComments("/* tak ditutup SELECT 1").trim(), "")
})

test("hasStatement membedakan buffer kosong dari yang berisi", () => {
  assert.equal(hasStatement("-- Write SQL here, or generate it from a question"), false)
  assert.equal(hasStatement("   \n\n  "), false)
  assert.equal(hasStatement("-- catatan\nSELECT 1"), true)
})

test("sqlSummary mengambil baris pertama dan memotongnya", () => {
  assert.equal(sqlSummary("-- catatan\nSELECT a\nFROM t"), "SELECT a")
  assert.equal(sqlSummary("SELECT " + "x".repeat(80), 20), "SELECT xxxxxxxxxxxx…")
})
