import { strict as assert } from "node:assert"
import { test } from "node:test"
import { EDITOR_SHORTCUTS } from "./editor-shortcuts"

test("EDITOR_SHORTCUTS tidak kosong", () => {
  assert.ok(EDITOR_SHORTCUTS.length > 0)
})

test("setiap entri memiliki action dan label yang tidak kosong", () => {
  for (const entry of EDITOR_SHORTCUTS) {
    assert.ok(entry.action.length > 0, `action kosong di entri: ${JSON.stringify(entry)}`)
    assert.ok(entry.label.length > 0, `label kosong di entri: ${JSON.stringify(entry)}`)
  }
})

test("tidak ada dua entri yang mengikat key yang sama", () => {
  const keys = EDITOR_SHORTCUTS.map((e) => e.key)
  const unique = new Set(keys)
  assert.equal(unique.size, keys.length, "ditemukan key duplikat")
})

test("setiap entri yang punya macKey berbeda dari key utamanya", () => {
  for (const entry of EDITOR_SHORTCUTS) {
    if (entry.macKey !== undefined) {
      assert.notEqual(
        entry.macKey,
        entry.key,
        `macKey sama dengan key di entri: ${JSON.stringify(entry)}`
      )
    }
  }
})
