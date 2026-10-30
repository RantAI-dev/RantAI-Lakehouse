import { strict as assert } from "node:assert"
import { test } from "node:test"
import { toCsv, toTsv } from "./csv"

/**
 * `downloadCsv` menyentuh DOM dan tidak diuji di sini; yang penting dan
 * mudah salah adalah aturan escaping-nya.
 */

test("toCsv menulis header dan baris", () => {
  const csv = toCsv(
    ["id", "name"],
    [
      { id: "1", name: "alpha" },
      { id: "2", name: "beta" },
    ]
  )

  // CRLF sesuai RFC 4180 supaya Excel di Windows tidak menyatukan baris.
  assert.equal(csv, "id,name\r\n1,alpha\r\n2,beta")
})

test("toCsv membungkus sel yang mengandung koma", () => {
  const csv = toCsv(["v"], [{ v: "a,b" }])

  assert.equal(csv, 'v\r\n"a,b"')
})

test("toCsv menggandakan tanda kutip di dalam sel", () => {
  // Ini kesalahan escaping paling umum: tanda kutip harus digandakan,
  // bukan di-escape dengan backslash.
  const csv = toCsv(["v"], [{ v: 'say "hi"' }])

  assert.equal(csv, 'v\r\n"say ""hi"""')
})

test("toCsv membungkus sel yang mengandung newline", () => {
  const csv = toCsv(["v"], [{ v: "line1\nline2" }])

  assert.equal(csv, 'v\r\n"line1\nline2"')
})

test("toCsv menulis sel kosong untuk nilai null dan undefined", () => {
  // Bukan string "null" — hasil unduhan harus terbaca sebagai data kosong.
  const csv = toCsv(["a", "b", "c"], [{ a: "x", b: null, c: undefined }])

  assert.equal(csv, "a,b,c\r\nx,,")
})

test("toCsv mengisi sel kosong saat kolom tidak ada di baris", () => {
  // Baris hasil query bisa saja tidak memuat semua kolom; jumlah sel per
  // baris tetap harus sama dengan jumlah kolom.
  const csv = toCsv(["a", "b"], [{ a: "x" }])

  assert.equal(csv, "a,b\r\nx,")
})

test("toCsv membungkus header yang mengandung koma", () => {
  // Nama kolom hasil query bisa mengandung koma, mis. alias buatan pengguna.
  const csv = toCsv(["total, gross"], [{ "total, gross": "1" }])

  assert.equal(csv, '"total, gross"\r\n1')
})

test("toCsv menghasilkan header saja ketika tidak ada baris", () => {
  assert.equal(toCsv(["a", "b"], []), "a,b")
})

// SEC-17: the same rule as `needs_apostrophe` in the API's csv_safe.rs.

test("toCsv memberi apostrof pada sel yang diawali karakter rumus", () => {
  for (const value of ["=1+1", "+cmd", "-cmd", "@SUM(A1)", "\tx"]) {
    assert.equal(toCsv(["v"], [{ v: value }]), `v\r\n'${value}`)
  }
})

test("toCsv memberi apostrof di dalam tanda kutip, bukan di luarnya", () => {
  assert.equal(
    toCsv(["v"], [{ v: '=HYPERLINK("http://example.invalid","x")' }]),
    `v\r\n"'=HYPERLINK(""http://example.invalid"",""x"")"`
  )
  assert.equal(toCsv(["v"], [{ v: "\rx" }]), 'v\r\n"\'\rx"')
})

test("toCsv membiarkan angka polos tetap angka", () => {
  for (const value of ["-5", "+3.2e4", ".5", "-.5", "5.", "-0.0", "-1.5E-3"]) {
    assert.equal(toCsv(["v"], [{ v: value }]), `v\r\n${value}`)
  }
})

test("toCsv memberi apostrof pada sel yang hanya mirip angka", () => {
  for (const value of ["-1+1", "-", "+", "-.", "-5e", "--5", "-5 ", "-5\n"]) {
    const csv = toCsv(["v"], [{ v: value }])
    assert.ok(csv.startsWith("v\r\n") && csv.slice(3).includes(`'${value}`), value)
  }
})

test("toCsv membiarkan sel biasa dan sel kosong", () => {
  assert.equal(toCsv(["a", "b", "c"], [{ a: "a=b", b: "x-y", c: "" }]), "a,b,c\r\na=b,x-y,")
})

test("toCsv memperlakukan header seperti sel lain", () => {
  assert.equal(toCsv(["=total", "@x"], []), "'=total,'@x")
})

// SEC-17: the tab-separated twin shares the formula guard with the comma
// one, so both are proven here.

test("toTsv menulis header dan baris dengan pemisah tab", () => {
  const tsv = toTsv(
    ["id", "name"],
    [
      { id: "1", name: "alpha" },
      { id: "2", name: "beta" },
    ]
  )

  // CRLF, as RFC 4180 says, the same as the comma version.
  assert.equal(tsv, "id\tname\r\n1\talpha\r\n2\tbeta")
})

test("toTsv membungkus sel yang mengandung tab", () => {
  // A tab inside a cell is the separator, so the cell has to be quoted.
  const tsv = toTsv(["v"], [{ v: "a\tb" }])

  assert.equal(tsv, 'v\r\n"a\tb"')
})

test("toTsv membungkus sel yang mengandung newline", () => {
  const tsv = toTsv(["v"], [{ v: "line1\nline2" }])

  assert.equal(tsv, 'v\r\n"line1\nline2"')
})

test("toTsv menggandakan tanda kutip di dalam sel", () => {
  const tsv = toTsv(["v"], [{ v: 'say "hi"' }])

  assert.equal(tsv, 'v\r\n"say ""hi"""')
})

test("toTsv memberi apostrof pada sel yang diawali karakter rumus", () => {
  for (const value of ["=1+1", "+cmd", "-cmd", "@SUM(A1)"]) {
    assert.equal(toTsv(["v"], [{ v: value }]), `v\r\n'${value}`)
  }
})

test("toTsv memberi apostrof di dalam tanda kutip, bukan di luarnya", () => {
  assert.equal(
    toTsv(["v"], [{ v: '=HYPERLINK("http://example.invalid","x")' }]),
    `v\r\n"'=HYPERLINK(""http://example.invalid"",""x"")"`
  )
})

test("toTsv menulis sel kosong untuk nilai null dan undefined", () => {
  const tsv = toTsv(["a", "b", "c"], [{ a: "x", b: null, c: undefined }])

  assert.equal(tsv, "a\tb\tc\r\nx\t\t")
})

test("toTsv memperlakukan header seperti sel lain", () => {
  assert.equal(toTsv(["=total", "@x"], []), "'=total\t'@x")
})

test("toTsv membiarkan koma di dalam sel apa adanya", () => {
  // A comma is not the separator in a tab file, so the cell needs no quotes.
  assert.equal(toTsv(["v"], [{ v: "a,b" }]), "v\r\na,b")
})

