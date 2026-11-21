import { strict as assert } from "node:assert"
import { test } from "node:test"
import { ServiceError } from "@/services/errors"
import { buildErrorDescription, readableErrorMessage } from "./notify-message"

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

// BI-18 part C review fix (SHOULD-FIX) R20: the API's `code: detail` text never reaches the reader.
test("a coded refusal becomes the one sentence, and the toast adds the next step", () => {
  const err = new ServiceError("permission_denied", "permission_denied: dashboard:write")
  assert.equal(
    buildErrorDescription(err),
    "You don't have access. This needs the dashboard:write permission. Ask a workspace admin."
  )
  assert.equal(
    readableErrorMessage("permission_denied: dashboard:write"),
    "You don't have access. This needs the dashboard:write permission."
  )
})

test("every known code prefix is dropped, and a message without one is left as it is", () => {
  for (const code of ["not_found", "invalid_request", "unavailable", "too_many_requests"]) {
    const text = readableErrorMessage(`${code}: something internal`)
    assert.ok(!text.includes(`${code}:`) && !text.includes("internal"), text)
  }
  assert.equal(readableErrorMessage("Chart not found."), "Chart not found.")
  assert.equal(readableErrorMessage("permissions: nothing"), "permissions: nothing")
  assert.ok(!buildErrorDescription(new ServiceError("invalid_request", "invalid_request: x"))?.includes("invalid_request:"))
})

// BI-18 part C review fix R20 (follow-up: keep the server's sentence).
test("a sentence after the code is shown as the server wrote it", () => {
  assert.equal(
    readableErrorMessage("permission_denied: Downloads are switched off for your role."),
    "Downloads are switched off for your role."
  )
  assert.equal(
    readableErrorMessage("unavailable: The PDF service is not reachable. Reference: abc."),
    "The PDF service is not reachable. Reference: abc."
  )
})

test("a code with nothing after it is the fixed sentence, and a fragment under another code stays hidden", () => {
  assert.equal(
    readableErrorMessage("permission_denied: dashboard:write"),
    "You don't have access. This needs the dashboard:write permission."
  )
  assert.equal(
    readableErrorMessage("permission_denied"),
    "You don't have access."
  )
  // A space but no closing full stop is not taken for a sentence.
  assert.ok(!readableErrorMessage("not_found: row 12 in table x").includes("row 12"))
})

test("a refusal the client already put into words keeps them and gains the next step once", () => {
  assert.equal(
    buildErrorDescription(
      new ServiceError(
        "permission_denied",
        "You don't have access. This needs the pipeline:write permission.",
        403
      )
    ),
    "You don't have access. This needs the pipeline:write permission. Ask a workspace admin."
  )
  assert.equal(
    buildErrorDescription(new ServiceError("permission_denied", "")),
    "You don't have access. Ask a workspace admin."
  )
})

test("a coded refusal a client filed under another code still reads as a refusal", () => {
  assert.equal(
    buildErrorDescription(
      new ServiceError("invalid_request", "permission_denied: connector:manage", 403)
    ),
    "You don't have access. This needs the connector:manage permission. Ask a workspace admin."
  )
})

test("a fragment after the refusal code is given as the reason", () => {
  assert.equal(
    readableErrorMessage("permission_denied: manage on this item"),
    "You don't have access. Reason given: manage on this item."
  )
})
