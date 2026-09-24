import { strict as assert } from "node:assert"
import { test } from "node:test"
import { compareValues, type SortableValue } from "./table-sort"

/** Membantu memastikan urutan akhir, bukan sekadar nilai balikan. */
function sorted(values: SortableValue[]): SortableValue[] {
  return [...values].sort(compareValues)
}

test("compareValues mengurutkan angka secara numerik", () => {
  // Perbandingan leksikografis akan menaruh 10 sebelum 9 — ini menjaga
  // supaya kolom seperti "Assets" terurut benar.
  assert.deepEqual(sorted([10, 2, 9, 1]), [1, 2, 9, 10])
})

test("compareValues mengurutkan string secara natural", () => {
  // Nama aset sering berakhiran angka; `numeric: true` menjaga asset_2
  // tetap sebelum asset_10.
  assert.deepEqual(sorted(["asset_10", "asset_2", "asset_1"]), [
    "asset_1",
    "asset_2",
    "asset_10",
  ])
})

test("compareValues menaruh null dan undefined di akhir", () => {
  // Baris tanpa data tidak boleh menyela baris yang berisi.
  assert.deepEqual(sorted(["b", null, "a", undefined]), [
    "a",
    "b",
    null,
    undefined,
  ])
})

test("compareValues menganggap dua nilai kosong setara", () => {
  assert.equal(compareValues(null, undefined), 0)
  assert.equal(compareValues(null, null), 0)
})

test("compareValues konsisten saat arah dibalik", () => {
  // Pembalikan arah di DataTable dilakukan dengan menegasikan hasil, jadi
  // fungsi ini harus antisimetris untuk nilai yang terisi.
  assert.equal(compareValues("a", "b") < 0, true)
  assert.equal(compareValues("b", "a") > 0, true)
  assert.equal(compareValues(1, 2) < 0, true)
  assert.equal(compareValues(2, 1) > 0, true)
})

test("compareValues menangani boolean", () => {
  // false diurutkan sebelum true karena "false" < "true" secara leksikografis.
  assert.deepEqual(sorted([true, false, true]), [false, true, true])
})
