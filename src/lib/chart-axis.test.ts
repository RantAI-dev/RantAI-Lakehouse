import { strict as assert } from "node:assert"
import { test } from "node:test"
import { decimalsFor, formatCompactNumber, formatNumber, monthlyAxisLabel } from "./chart-axis"

const months = (from: number, to: number) =>
  Array.from({ length: (to - from + 1) * 12 }, (_, i) => `${from + Math.floor(i / 12)}-${String((i % 12) + 1).padStart(2, "0")}`)

test("monthlyAxisLabel hanya memberi label di awal tiap tahun", () => {
  const cats = months(2020, 2024)
  const label = monthlyAxisLabel(cats)
  assert.ok(label)
  const shown = cats.filter((c, i) => label.interval(i, c)).map(label.formatter)
  assert.deepEqual(shown, ["2020", "2021", "2022", "2023", "2024"])
})

test("monthlyAxisLabel tidak menyentuh sumbu pendek atau bukan bulanan", () => {
  assert.equal(monthlyAxisLabel(months(2024, 2024)), null)
  assert.equal(monthlyAxisLabel(Array.from({ length: 30 }, (_, i) => `Kota ${i}`)), null)
})

test("formatCompactNumber memakai singkatan Inggris", () => {
  assert.equal(formatCompactNumber(3_500_000), "3.5M")
  assert.equal(formatCompactNumber(300_000), "300K")
  assert.equal(formatCompactNumber(950), "950")
})

// BI-8 review fix (SHOULD-FIX) R4: decimals follow the drawn values.

test("whole-number data keeps whole-number ticks and labels", () => {
  assert.equal(decimalsFor([0, 12, 40_000, -5]), 0)
  assert.equal(formatCompactNumber(1234, 0), "1.2K")
  assert.equal(formatCompactNumber(950, 0), "950")
  assert.equal(formatNumber(1234567, 0), "1,234,567")
})

test("shares between 0 and 1 get three decimals so 0.18 and 0.23 differ", () => {
  const d = decimalsFor([0.23, 0.18, 0.32, 0.27])
  assert.equal(d, 3)
  assert.equal(formatCompactNumber(0.05, d), "0.05")
  assert.equal(formatCompactNumber(0, d), "0")
  assert.equal(formatCompactNumber(0.325, d), "0.325")
  assert.notEqual(formatCompactNumber(0.18, d), formatCompactNumber(0.23, d))
})

test("fractions under 100 get two decimals, under 1,000 one, from 1,000 none", () => {
  assert.equal(decimalsFor([1.5, 7.25, 3]), 2)
  assert.equal(formatNumber(7.254, 2), "7.25")
  assert.equal(decimalsFor([12.5, 99.1]), 2)
  assert.equal(decimalsFor([12.5, 140.25]), 1)
  assert.equal(decimalsFor([1234.5, 2.5]), 0)
  assert.equal(formatCompactNumber(1234.5, 0), "1.2K")
})

test("negatives and non-finite values are handled", () => {
  assert.equal(decimalsFor([-0.5, -0.25]), 3)
  assert.equal(formatCompactNumber(-0.25, 3), "-0.25")
  assert.equal(formatCompactNumber(-1500, 0), "-1.5K")
  assert.equal(decimalsFor([Number.NaN, Infinity, 3]), 0)
  assert.equal(decimalsFor([]), 0)
})
