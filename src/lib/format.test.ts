import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  formatBytes,
  formatCompactNumber,
  formatCost,
  formatDuration,
  formatLagSeconds,
  formatPercent,
  formatRelativeTime,
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
