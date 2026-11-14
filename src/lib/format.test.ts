import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  asSentence,
  formatBytes,
  formatLocaleNumber,
  formatCompactNumber,
  formatCost,
  formatDuration,
  formatLagSeconds,
  formatPercent,
  formatRelativeTime,
  formatTimeUntil,
  isPast,
  parseTimestamp,
} from "./format"

test("formatBytes", () => {
  assert.equal(formatBytes(0), "0 B")
  assert.equal(formatBytes(1536), "1.5 KB")
  assert.match(formatBytes(2.4 * 1024 ** 4), /TB/)
})

test("formatDuration", () => {
  assert.equal(formatDuration(840), "840 ms")
  assert.equal(formatDuration(2400), "2.4 s")
})

test("formatCost / percent / lag / compact", () => {
  assert.equal(formatCost(0.0421), "0.0421 cu")
  assert.equal(formatPercent(0.634), "63.4%")
  assert.equal(formatLagSeconds(8), "8 s")
  assert.equal(formatLagSeconds(125), "2m")
  assert.equal(formatLagSeconds(36 * 3600), "36h 00m")
  assert.equal(formatLagSeconds(194 * 3600 + 240), "8d 2h")
  assert.match(formatCompactNumber(1_234_567), /M|1\.2/)
})

test("parseTimestamp membaca DateTime ClickHouse tanpa zona sebagai UTC", () => {
  // ClickHouse berjalan di UTC; tanpa ini, di WIB semua cap waktu tampil 7 jam lebih tua.
  assert.equal(parseTimestamp("2026-09-13 07:09:40").toISOString(), "2026-09-13T07:09:40.000Z")
  assert.equal(parseTimestamp("2026-09-13T07:09:40+07:00").toISOString(), "2026-09-13T00:09:40.000Z")
})

test("formatRelativeTime untuk sesi Copilot yang baru disimpan", () => {
  const now = Date.parse("2026-09-13T07:12:40Z")
  assert.equal(formatRelativeTime("2026-09-13 07:09:40", now), "3m ago")
})

test("isPast membedakan jadwal yang sudah lewat dari yang akan datang", () => {
  const now = Date.parse("2026-09-20T10:00:00Z")
  assert.equal(isPast("2026-09-20T09:00:00Z", now), true)
  assert.equal(isPast("2026-09-20T11:00:00Z", now), false)
  // Tidak ada jadwal bukan berarti terlambat.
  assert.equal(isPast(null, now), false)
  assert.equal(isPast("bukan tanggal", now), false)
})

test("formatTimeUntil untuk jadwal ingest berikutnya", () => {
  const now = Date.parse("2026-09-30T01:30:00Z")
  assert.equal(formatTimeUntil("2026-09-30T01:34:10Z", now), "in 5m")
  assert.equal(formatTimeUntil("2026-09-30T04:00:00Z", now), "in 2h")
  assert.equal(formatTimeUntil("2026-10-02T02:00:00Z", now), "in 2d")
  // Sudah lewat: run-nya sedang dimulai.
  assert.equal(formatTimeUntil("2026-09-30T01:00:00Z", now), "now")
  assert.equal(formatTimeUntil(null, now), "—")
})

// `BI-58` T6.
test("formatLocaleNumber groups thousands in the browser's locale and reads a missing number as a dash", () => {
  assert.equal(formatLocaleNumber(1571564), (1571564).toLocaleString())
  assert.equal(formatLocaleNumber(1571564).replace(/\D/g, ""), "1571564")
  assert.notEqual(formatLocaleNumber(1571564), "1571564")
  assert.equal(formatLocaleNumber(null), "—")
  assert.equal(formatLocaleNumber(Number.NaN), "—")
})

test("asSentence raises the first letter of a server's error and ends it with a full stop", () => {
  assert.equal(asSentence("invalid or missing measure column"), "Invalid or missing measure column.")
  assert.equal(asSentence("Preview failed."), "Preview failed.")
  assert.equal(asSentence("  a parameter can stand for a column (not a pivot)."), "A parameter can stand for a column (not a pivot).")
  assert.equal(asSentence(""), "")
})
