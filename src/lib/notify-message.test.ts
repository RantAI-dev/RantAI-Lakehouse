import { strict as assert } from "node:assert"
import { test } from "node:test"
import { ServiceError } from "@/services/errors"
import { buildErrorDescription } from "./notify-message"

/**
 * Mengikuti konvensi repo (`node:test` + `node:assert`, seperti
 * `format.test.ts`). Modul yang diuji sengaja bebas dari `sonner` sehingga
 * bisa dijalankan tanpa DOM.
 */

test("buildErrorDescription menggabungkan pesan asli dengan saran langkah lanjut", () => {
  const description = buildErrorDescription(
    new ServiceError("permission_denied", "Forbidden.")
  )

  // Detail dari server tetap ditampilkan, tapi ditambah langkah yang bisa
  // ditindaklanjuti — bukan sekadar kode error.
  assert.ok(description?.includes("Forbidden."))
  assert.ok(description?.includes("Ask a workspace admin."))
})

test("buildErrorDescription memberi saran berbeda per kode error", () => {
  assert.ok(
    buildErrorDescription(new ServiceError("not_found", "Missing."))?.includes(
      "Reload the list."
    )
  )
  assert.ok(
    buildErrorDescription(
      new ServiceError("invalid_request", "Bad input.")
    )?.includes("Check the input")
  )
  assert.ok(
    buildErrorDescription(new ServiceError("unavailable", "Down."))?.includes(
      "Try again shortly."
    )
  )
})

test("buildErrorDescription mengembalikan null untuk error aborted", () => {
  // Pembatalan adalah alur normal (unmount / panggilan ditimpa), bukan
  // kegagalan yang perlu dilaporkan ke pengguna.
  assert.equal(
    buildErrorDescription(
      new ServiceError("aborted", "The request was cancelled.")
    ),
    null
  )
})

test("buildErrorDescription memperlakukan AbortError DOM sebagai aborted", () => {
  // `toServiceError` memetakan DOMException AbortError ke kode `aborted`,
  // jadi pembatalan lewat AbortController juga harus senyap.
  const abortError = new DOMException("Aborted", "AbortError")
  assert.equal(buildErrorDescription(abortError), null)
})

test("buildErrorDescription menangani nilai lempar non-Error", () => {
  // Nilai apa pun bisa dilempar di JavaScript; jangan sampai penanganannya
  // ikut melempar.
  const description = buildErrorDescription("boom")

  assert.ok(description)
  assert.ok(description.includes("Try again shortly."))
})

test("buildErrorDescription tetap memberi teks saat pesan error kosong", () => {
  const description = buildErrorDescription(new ServiceError("not_found", "  "))

  // Pesan kosong tidak boleh menghasilkan toast tanpa keterangan sama sekali.
  assert.equal(description, "It may already have been deleted. Reload the list.")
})
