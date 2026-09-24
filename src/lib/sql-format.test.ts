import { strict as assert } from "node:assert"
import { test } from "node:test"
import { formatSql } from "./sql-format"

test("formatSql memecah klausa utama ke barisnya sendiri", () => {
  assert.equal(
    formatSql("select a, b from t where a > 1 group by a order by b limit 10"),
    "SELECT a, b\nFROM t\nWHERE a > 1\nGROUP BY a\nORDER BY b\nLIMIT 10"
  )
})

test("formatSql membesarkan kata kunci sebaris tanpa memecah baris", () => {
  assert.equal(
    formatSql("select a from t where a = 1 and b is null"),
    "SELECT a\nFROM t\nWHERE a = 1 AND b IS NULL"
  )
})

test("formatSql tidak menyentuh isi string, identifier, dan komentar", () => {
  assert.equal(
    formatSql("select 'from where' as x from t"),
    "SELECT 'from where' AS x\nFROM t"
  )
  assert.equal(formatSql('select "select" from t'), 'SELECT "select"\nFROM t')
  assert.equal(
    formatSql("-- from where\nselect 1"),
    "-- from where\nSELECT 1"
  )
})

test("formatSql menjaga tanda baca tetap rapat", () => {
  assert.equal(
    formatSql("select count(a), sum(b) from t"),
    "SELECT count(a), sum(b)\nFROM t"
  )
})

test("formatSql membiarkan buffer kosong apa adanya", () => {
  assert.equal(formatSql("   "), "   ")
})
