import { strict as assert } from "node:assert"
import { test } from "node:test"
import { EDITOR_SHORTCUTS, isApplePlatform, studioKeys } from "./editor-shortcuts"

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

// UX-9 review: the status bar names the keys of the person's own platform.
test("isApplePlatform reads ⌘ on a Mac or an iPad and Ctrl elsewhere, and studioKeys follows it", () => {
  assert.equal(isApplePlatform({ platform: "MacIntel" }), true)
  assert.equal(isApplePlatform({ userAgentData: { platform: "macOS" } }), true)
  assert.equal(isApplePlatform({ platform: "iPad" }), true)
  assert.equal(isApplePlatform({ platform: "Win32" }), false)
  assert.equal(isApplePlatform({ userAgentData: { platform: "Linux" }, platform: "" }), false)
  assert.deepEqual(studioKeys(true), { run: "⌘↵", commands: "⌘⇧P" })
  assert.deepEqual(studioKeys(false), { run: "Ctrl+↵", commands: "Ctrl+Shift+P" })
})
