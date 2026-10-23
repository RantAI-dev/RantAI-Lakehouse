import { strict as assert } from "node:assert"
import { test } from "node:test"
import { catalogRefusal } from "./catalog-refusal"

test("catalogRefusal membaca jawaban supported: false beserta alasannya", () => {
  const refusal = catalogRefusal({ supported: false, reason: "Tenant belum dikonfigurasi" })

  assert.deepEqual(refusal, { reason: "Tenant belum dikonfigurasi" })
})

test("catalogRefusal tetap menolak ketika alasan tidak dikirim, tanpa mengarang alasan spesifik", () => {
  const refusal = catalogRefusal({ supported: false })

  assert.notEqual(refusal, null)
  assert.ok((refusal?.reason ?? "").length > 0)
})

test("catalogRefusal tidak menolak detail aset yang normal", () => {
  assert.equal(catalogRefusal({ id: "silver.orders", schema: [] }), null)
  assert.equal(catalogRefusal({ supported: true, schema: [] }), null)
})

test("catalogRefusal tidak melempar untuk badan yang bukan objek", () => {
  assert.equal(catalogRefusal(null), null)
  assert.equal(catalogRefusal(undefined), null)
  assert.equal(catalogRefusal("supported: false"), null)
})
