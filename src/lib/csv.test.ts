import { strict as assert } from "node:assert"
import { test } from "node:test"
import { toCsv } from "./csv"

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
