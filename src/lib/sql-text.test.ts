import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  hasStatement,
  MAX_STATEMENTS_PER_RUN,
  splitStatements,
  sqlSummary,
  stripSqlComments,
  tokenizeSql,
} from "./sql-text"

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

test("stripSqlComments tidak menyentuh tanda komentar di dalam string", () => {
  const sql = "SELECT '-- bukan komentar' AS a"
  assert.equal(stripSqlComments(sql), sql)
  assert.equal(
    stripSqlComments("SELECT '/* juga bukan */' AS a"),
    "SELECT '/* juga bukan */' AS a"
  )
})

test("tokenizeSql memberi jenis dan posisi setiap token", () => {
  const sql = "select 'a b' -- c"
  const tokens = tokenizeSql(sql)
  assert.deepEqual(
    tokens.map((token) => [token.kind, token.text]),
    [
      ["word", "select"],
      ["quoted", "'a b'"],
      ["comment", "-- c"],
    ]
  )
  assert.deepEqual(
    tokens.map((token) => [token.from, token.to]),
    [
      [0, 6],
      [7, 12],
      [13, 17],
    ]
  )
  for (const token of tokens) {
    assert.equal(sql.slice(token.from, token.to), token.text)
  }
})

test("splitStatements tidak memecah titik koma di dalam string", () => {
  const sql = "SELECT 'a;b';\nSELECT 2"
  const statements = splitStatements(sql)
  assert.equal(statements.length, 2)
  assert.equal(statements[0]?.text, "SELECT 'a;b'")
  assert.equal(statements[1]?.text, "SELECT 2")
})

test("splitStatements tidak memecah titik koma di dalam identifier tanda kutip", () => {
  const sql = 'SELECT "a;b";\nSELECT 2'
  const statements = splitStatements(sql)
  assert.equal(statements.length, 2)
  assert.equal(statements[0]?.text, 'SELECT "a;b"')
  assert.equal(statements[1]?.text, "SELECT 2")
})

test("splitStatements tidak memecah titik koma di dalam identifier backtick", () => {
  const sql = "SELECT `a;b`;\nSELECT 2"
  const statements = splitStatements(sql)
  assert.equal(statements.length, 2)
  assert.equal(statements[0]?.text, "SELECT `a;b`")
  assert.equal(statements[1]?.text, "SELECT 2")
})

test("splitStatements tidak memecah titik koma di dalam komentar baris", () => {
  const sql = "SELECT 1 -- a;b\n; SELECT 2"
  const statements = splitStatements(sql)
  assert.equal(statements.length, 2)
  assert.equal(statements[0]?.text, "SELECT 1")
  assert.equal(statements[1]?.text, "SELECT 2")
})

test("splitStatements tidak memecah titik koma di dalam komentar blok", () => {
  const sql = "SELECT 1 /* a;b */; SELECT 2"
  const statements = splitStatements(sql)
  assert.equal(statements.length, 2)
  assert.equal(statements[0]?.text, "SELECT 1")
  assert.equal(statements[1]?.text, "SELECT 2")
})

test("splitStatements tidak membuat pernyataan kosong dari titik koma di akhir", () => {
  assert.equal(splitStatements("SELECT 1;").length, 1)
  assert.equal(splitStatements("SELECT 1;").map((s) => s.text).join("|"), "SELECT 1")
  assert.equal(splitStatements("SELECT 1; SELECT 2;").length, 2)
})

test("splitStatements memberi rentang yang dipotong kembali persis ke teksnya", () => {
  const sql = "SELECT 1;\nSELECT 2;\nSELECT 3"
  const statements = splitStatements(sql)
  assert.equal(statements.length, 3)
  for (const statement of statements) {
    assert.equal(sql.slice(statement.from, statement.to), statement.text)
  }
  assert.deepEqual(
    statements.map((statement) => statement.text),
    ["SELECT 1", "SELECT 2", "SELECT 3"]
  )
})

test("splitStatements membuang potongan yang hanya berisi komentar", () => {
  assert.equal(splitStatements("-- catatan\n").length, 0)
  const sql = "SELECT 1; -- catatan\n; SELECT 2"
  const statements = splitStatements(sql)
  assert.equal(statements.length, 2)
  assert.equal(statements[0]?.text, "SELECT 1")
  assert.equal(statements[1]?.text, "SELECT 2")
})

test("splitStatements tidak menghasilkan pernyataan dari input kosong", () => {
  assert.equal(splitStatements("").length, 0)
  assert.equal(splitStatements("   \n  ").length, 0)
  assert.equal(splitStatements(";").length, 0)
})

test("splitStatements memotong rentang ke pernyataan, bukan ke komentar di sekitarnya", () => {
  const sql = "-- lead\nSELECT 1 -- tail"
  const statements = splitStatements(sql)
  assert.equal(statements.length, 1)
  assert.equal(statements[0]?.text, "SELECT 1")
  assert.equal(sql.slice(statements[0].from, statements[0].to), "SELECT 1")
})

test("splitStatements dan stripSqlComments sepakat soal isi string", () => {
  const sql = "SELECT '-- bukan komentar; bukan pemisah' AS a"
  assert.equal(stripSqlComments(sql), sql)
  const statements = splitStatements(sql)
  assert.equal(statements.length, 1)
  assert.equal(statements[0]?.text, sql)
})

test("MAX_STATEMENTS_PER_RUN adalah batas dua puluh statement per run", () => {
  assert.equal(MAX_STATEMENTS_PER_RUN, 20)
})
