import { describe, expect, test } from "bun:test"
import { decimalsFor, formatCompactNumber, formatNumber } from "./chart-axis"
import { cellContent } from "./cell-format"
import { formatAxisValue, formatCompactCurrency, formatDateValue, formatValue } from "./value-format"

describe("formatValue", () => {
  test("a compact rupiah amount in id-ID reads Rp 1,23 M", () => {
    const fmt = { style: "compact", currency: "IDR", decimals: 2 } as const
    expect(formatValue(1234567.891, fmt, "id-ID")).toBe("Rp 1,23 M")
  })

  test("a compact dollar amount in en-US reads $1.23 M", () => {
    const fmt = { style: "compact", currency: "USD", decimals: 2 } as const
    expect(formatValue(1234567.891, fmt, "en-US")).toBe("$1.23 M")
  })

  test("a currency written in full follows the locale and the display", () => {
    expect(formatValue(1234.5, { style: "currency", currency: "USD" }, "en-US")).toBe("$1,234.50")
    expect(formatValue(1234.5, { style: "currency", currency: "USD", currencyDisplay: "code" }, "en-US")).toBe("USD 1,234.50")
    expect(formatValue(1234, { style: "currency", currency: "IDR", decimals: 0 }, "id-ID")).toBe("Rp 1.234")
  })

  test("a percent multiplies by 100", () => {
    expect(formatValue(0.125, { style: "percent", decimals: 1 })).toBe("12.5%")
    expect(formatValue(0.5, { style: "percent", decimals: 0 })).toBe("50%")
  })

  test("a number that already holds a percentage uses number with a suffix", () => {
    expect(formatValue(12.5, { style: "number", decimals: 1, suffix: "%" })).toBe("12.5%")
  })

  test("scientific notation", () => {
    expect(formatValue(1234567, { style: "scientific", decimals: 2 })).toBe("1.23E6")
  })

  test("prefix and suffix are written around the number as given", () => {
    expect(formatValue(5, { style: "number", prefix: "~ ", suffix: " pcs" })).toBe("~ 5 pcs")
  })

  test("thousands off removes the separator, thousands on follows the locale", () => {
    expect(formatValue(1234567.5, { style: "number", decimals: 1, thousands: false })).toBe("1234567.5")
    expect(formatValue(1234567.5, { style: "number", decimals: 1 }, "id-ID")).toBe("1.234.567,5")
  })

  test("compact uses K, M, B and T in every locale", () => {
    expect(formatValue(1200, { style: "compact" }, "id-ID")).toBe("1,2 K")
    expect(formatValue(3.4e9, { style: "compact" })).toBe("3.4 B")
    expect(formatValue(2e12, { style: "compact" })).toBe("2 T")
    expect(formatValue(999, { style: "compact" })).toBe("999")
    expect(formatValue(-1500, { style: "compact" })).toBe("-1.5 K")
  })

  test("ten decimals are kept and a value that is not finite is returned as text", () => {
    expect(formatValue(1 / 3, { style: "number", decimals: 10 })).toBe("0.3333333333")
    expect(formatValue(Number.NaN, { style: "number" })).toBe("NaN")
  })

  test("auto, and no format at all, give what the console showed before", () => {
    for (const v of [0, 7, 1234, 1234567, 0.236]) {
      const d = decimalsFor([v])
      expect(formatValue(v, undefined, "en-US", d)).toBe(formatNumber(v, d))
      expect(formatValue(v, { style: "auto" }, "en-US", d)).toBe(formatNumber(v, d))
    }
  })
})

describe("formatAxisValue", () => {
  test("auto keeps the compact 1.2K of before", () => {
    for (const v of [5, 1200, 3_500_000, 2e9]) {
      expect(formatAxisValue(v, undefined)).toBe(formatCompactNumber(v, 0))
    }
  })

  test("a currency axis shortens large amounts and honours set decimals", () => {
    expect(formatAxisValue(2_500_000, { style: "currency", currency: "IDR" }, "id-ID")).toBe("Rp 2,5 M")
    expect(formatAxisValue(2_500_000, { style: "currency", currency: "IDR", decimals: 0 }, "id-ID")).toBe("Rp 2.500.000")
    expect(formatCompactCurrency(1500, { style: "compact", currency: "USD" })).toBe("$1.5 K")
  })
})

describe("formatDateValue", () => {
  test("auto returns the text as it came", () => {
    expect(formatDateValue("2026-10-15", undefined)).toBe("2026-10-15")
    expect(formatDateValue("not a date", { style: "long" })).toBe("not a date")
  })

  test("each style, in the report day and not the browser's", () => {
    expect(formatDateValue("2026-10-05", { style: "iso" })).toBe("2026-10-05")
    expect(formatDateValue("2026-10-05", { style: "medium" })).toBe("Oct 5, 2026")
    expect(formatDateValue("2026-10-05", { style: "long" })).toBe("October 5, 2026")
    expect(formatDateValue("2026-10-05", { style: "long", abbreviate: true })).toBe("Oct 5, 2026")
    expect(formatDateValue("2026-10-05", { style: "monthYear" })).toBe("October 2026")
    expect(formatDateValue("2026-10-05", { style: "short" })).toBe("10/05/26")
  })

  test("a timestamp takes 12- or 24-hour time", () => {
    expect(formatDateValue("2026-10-05 14:05:09", { style: "iso", hour12: false })).toBe("2026-10-05 14:05")
    expect(formatDateValue("2026-10-05 14:05:09", { style: "iso", hour12: true })).toBe("2026-10-05 2:05 PM")
  })
})

describe("cellContent with a locale and a currency code", () => {
  test("a currency column with no code stays Indonesian rupiah, whatever the locale", () => {
    expect(cellContent(1234, { format: "currency" }, "en-US")).toEqual(cellContent(1234, { format: "currency" }))
  })

  test("a currency column with a code follows the locale", () => {
    expect(cellContent(1234.5, { format: "currency", currency: "USD", decimals: 2 }, "en-US")).toEqual({
      kind: "text",
      text: "$1,234.50",
    })
  })

  test("a number column follows the locale and ten decimals are allowed", () => {
    expect(cellContent(1234.5, { format: "number" }, "id-ID")).toEqual({ kind: "text", text: "1.234,5" })
    expect(cellContent(0.5, { format: "number", decimals: 10 })).toEqual({ kind: "text", text: "0.5000000000" })
  })
})
