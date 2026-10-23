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

test("formatSql tanpa pilihan sama dengan pilihan baku", () => {
  const sql = "select a, b from t where a > 1 group by a order by b limit 10"
  assert.equal(formatSql(sql), formatSql(sql, { keywordCase: "upper", indent: 0 }))
})

test("formatSql menulis kata kunci huruf kecil saat diminta", () => {
  assert.equal(
    formatSql("select a from t where a = 1 and b is null", { keywordCase: "lower" }),
    "select a\nfrom t\nwhere a = 1 and b is null"
  )
})

test("formatSql mempertahankan kata kunci seperti yang diketik", () => {
  assert.equal(
    formatSql("Group By a from t", { keywordCase: "preserve" }),
    "Group By a\nfrom t"
  )
  assert.equal(
    formatSql("group by a FROM t", { keywordCase: "preserve" }),
    "group by a\nFROM t"
  )
  assert.equal(
    formatSql("select a from t where b is null", { keywordCase: "preserve" }),
    "select a\nfrom t\nwhere b is null"
  )
})

test("formatSql menggeser badan klausa ke baris berikutnya saat indent diaktifkan", () => {
  assert.equal(
    formatSql("select a, b from t where a > 1", { indent: 2 }),
    "SELECT\n  a, b\nFROM\n  t\nWHERE\n  a > 1"
  )
  assert.equal(
    formatSql("select a, b from t where a > 1", { indent: 4 }),
    "SELECT\n    a, b\nFROM\n    t\nWHERE\n    a > 1"
  )
})

test("formatSql menggeser klausa di dalam kurung satu tingkat lebih dalam", () => {
  assert.equal(
    formatSql("select * from (select a from b)", { indent: 2 }),
    "SELECT\n  *\nFROM\n  (\n    SELECT\n      a\n    FROM\n      b)"
  )
})

test("formatSql tidak menyentuh string, identifier, dan komentar di mode mana pun", () => {
  const sql = "select 'from where' as x from t -- and or"
  assert.equal(
    formatSql(sql, { keywordCase: "upper", indent: 0 }),
    "SELECT 'from where' AS x\nFROM t -- and or"
  )
  assert.equal(
    formatSql(sql, { keywordCase: "lower", indent: 0 }),
    "select 'from where' as x\nfrom t -- and or"
  )
  assert.equal(
    formatSql(sql, { keywordCase: "preserve", indent: 2 }),
    "select\n  'from where' as x\nfrom\n  t -- and or"
  )
  assert.equal(
    formatSql('select "select" from t', { keywordCase: "lower", indent: 4 }),
    'select\n    "select"\nfrom\n    t'
  )
})
