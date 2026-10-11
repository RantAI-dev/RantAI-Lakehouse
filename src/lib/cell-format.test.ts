import assert from "node:assert/strict"
import test from "node:test"

import { asNumber, cellContent, cellText, formatDate, safeHttpUrl, safeImageUrl } from "./cell-format"

test("a link cell is an anchor only for an http or https address", () => {
  assert.deepEqual(cellContent("https://example.com/a?b=1", { format: "link" }), {
    kind: "link", href: "https://example.com/a?b=1", text: "https://example.com/a?b=1",
  })
  assert.equal(cellContent("http://example.com", { format: "link" }).kind, "link")
  for (const bad of ["javascript:alert(1)", "data:text/html,x", "ftp://example.com", "//example.com", "example.com", "  ", "vbscript:x"]) {
    assert.deepEqual(cellContent(bad, { format: "link" }), { kind: "text", text: bad }, bad)
  }
  assert.equal(safeHttpUrl(42), null)
})

test("an image cell loads only an https address", () => {
  assert.equal(cellContent("https://example.com/a.png", { format: "image" }).kind, "image")
  for (const bad of ["http://example.com/a.png", "javascript:x", "data:image/png;base64,AAAA", "/a.png"]) {
    assert.deepEqual(cellContent(bad, { format: "image" }), { kind: "text", text: bad }, bad)
  }
  assert.equal(safeImageUrl("http://example.com/a.png"), null)
})

test("numbers read in the console's English convention and the column's digits", () => {
  assert.equal(cellText(1234567.891, { format: "number", decimals: 1 }), "1,234,567.9")
  assert.equal(cellText(1234, undefined), "1,234")
  assert.equal(cellText("1234", { format: "number", decimals: 0 }), "1,234")
  assert.equal(cellText(0.256, { format: "percent", decimals: 1 }), "25.6%")
  assert.match(cellText(1500, { format: "currency" }), /^Rp\s?1\.500$/)
  assert.match(cellText(1500.5, { format: "currency", decimals: 2 }), /^Rp\s?1\.500,50$/)
})

test("a value a number format cannot read is shown as it is, not as zero", () => {
  assert.equal(cellText("n/a", { format: "number" }), "n/a")
  assert.equal(cellText("n/a", { format: "currency" }), "n/a")
  assert.equal(cellText("abc", { format: "percent" }), "abc")
  assert.equal(asNumber(""), null)
  assert.equal(asNumber("12"), 12)
  assert.equal(asNumber(Number.NaN), null)
})

test("a date shows its calendar day whatever the browser's zone", () => {
  assert.equal(formatDate("2026-03-05"), "05 Mar 2026")
  assert.equal(formatDate("2026-03-05 23:59:59"), "05 Mar 2026")
  assert.equal(formatDate("not a date"), "not a date")
  assert.equal(cellText("2026-12-31", { format: "date" }), "31 Dec 2026")
})

test("an empty cell is empty, and an unset format leaves text alone", () => {
  assert.equal(cellText(null), "")
  assert.equal(cellText(undefined, { format: "currency" }), "")
  assert.equal(cellText("Bali"), "Bali")
})
