import { strict as assert } from "node:assert"
import { test } from "node:test"
import { formatCompactNumber, monthlyAxisLabel } from "./chart-axis"

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
