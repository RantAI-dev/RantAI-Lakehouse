import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  DEFAULT_FORMAT_SETTINGS,
  SQL_FORMAT_SETTINGS_KEY,
  parseFormatSettings,
  readFormatSettings,
  writeFormatSettings,
} from "./sql-format-settings"

test("parseFormatSettings memakai bawaan saat JSON rusak", () => {
  assert.deepEqual(parseFormatSettings("{bukan json"), DEFAULT_FORMAT_SETTINGS)
  assert.deepEqual(parseFormatSettings(null), DEFAULT_FORMAT_SETTINGS)
  assert.deepEqual(parseFormatSettings(""), DEFAULT_FORMAT_SETTINGS)
})

test("parseFormatSettings memakai bawaan saat yang tersimpan bukan objek", () => {
  assert.deepEqual(parseFormatSettings("42"), DEFAULT_FORMAT_SETTINGS)
  assert.deepEqual(parseFormatSettings('"teks"'), DEFAULT_FORMAT_SETTINGS)
  assert.deepEqual(parseFormatSettings("null"), DEFAULT_FORMAT_SETTINGS)
})

test("parseFormatSettings memakai bawaan untuk huruf kata kunci yang tidak dikenal", () => {
  assert.deepEqual(parseFormatSettings(JSON.stringify({ keywordCase: "title", indent: 2 })), {
    keywordCase: "upper",
    indent: 2,
  })
})

test("parseFormatSettings memakai bawaan untuk indent yang tidak dikenal", () => {
  assert.deepEqual(parseFormatSettings(JSON.stringify({ keywordCase: "lower", indent: 3 })), {
    keywordCase: "lower",
    indent: 0,
  })
})

test("parseFormatSettings memakai bawaan untuk field yang hilang", () => {
  assert.deepEqual(parseFormatSettings(JSON.stringify({})), DEFAULT_FORMAT_SETTINGS)
  assert.deepEqual(parseFormatSettings(JSON.stringify({ keywordCase: "preserve" })), {
    keywordCase: "preserve",
    indent: 0,
  })
})

test("pengaturan yang tersimpan kembali utuh", () => {
  const settings = { keywordCase: "preserve", indent: 4 } as const
  assert.deepEqual(parseFormatSettings(JSON.stringify(settings)), settings)
})

test("pengaturan dengan satu field buruk menyimpan field yang baik", () => {
  assert.deepEqual(parseFormatSettings(JSON.stringify({ keywordCase: "lower", indent: "dua" })), {
    keywordCase: "lower",
    indent: 0,
  })
})

test("penyimpanan pengaturan menulis dan membaca kembali dari localStorage", () => {
  window.localStorage.clear()
  writeFormatSettings({ keywordCase: "lower", indent: 4 })
  assert.equal(
    window.localStorage.getItem(SQL_FORMAT_SETTINGS_KEY),
    JSON.stringify({ keywordCase: "lower", indent: 4 })
  )
  assert.deepEqual(readFormatSettings(), { keywordCase: "lower", indent: 4 })
  window.localStorage.clear()
})

test("pembacaan dan penulisan tidak menjatuhkan halaman saat storage menolak", () => {
  const storage = window.localStorage
  const getItem = storage.getItem
  const setItem = storage.setItem
  storage.getItem = () => {
    throw new Error("storage is not available")
  }
  storage.setItem = () => {
    throw new Error("storage is not available")
  }
  try {
    assert.deepEqual(readFormatSettings(), DEFAULT_FORMAT_SETTINGS)
    writeFormatSettings({ keywordCase: "lower", indent: 2 })
  } finally {
    storage.getItem = getItem
    storage.setItem = setItem
  }
})
