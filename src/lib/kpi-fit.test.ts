import { describe, expect, it } from "bun:test"
import { KPI_FIT_FLOOR, KPI_FIT_NATURAL, compactKpiText, fitKpi, sameKpiFit } from "./kpi-fit"

// Owner QA 2026-11-15: "13,330,172" was cut on both sides in a 310 px KPI tile.
describe("fitKpi: the full number first, smaller next, compact last", () => {
  it("keeps the natural size when the full number fits", () => {
    expect(fitKpi(300, 200, 80)).toEqual(KPI_FIT_NATURAL)
    expect(fitKpi(200, 200, 80)).toEqual(KPI_FIT_NATURAL)
  })

  it("shrinks the full number to the room, down to the floor", () => {
    const fit = fitKpi(150, 200, 80)
    expect(fit.mode).toBe("full")
    expect(fit.scale).toBeCloseTo(0.75)
    expect(fitKpi(200 * KPI_FIT_FLOOR, 200, 80)).toEqual({ mode: "full", scale: KPI_FIT_FLOOR })
  })

  it("switches to the compact form only below the floor, at the size that fits it", () => {
    // 13,330,172 at 36 px is about 196 px; a 100 px room is half of it.
    expect(fitKpi(100, 196, 90)).toEqual({ mode: "compact", scale: 1 })
    const tight = fitKpi(60, 196, 90)
    expect(tight.mode).toBe("compact")
    expect(tight.scale).toBeCloseTo(60 / 90)
  })

  it("never goes under the floor, even when the compact form is still too wide", () => {
    expect(fitKpi(30, 196, 90)).toEqual({ mode: "compact", scale: KPI_FIT_FLOOR })
  })

  it("stays full at the floor when the value has no shorter form", () => {
    expect(fitKpi(50, 196, null)).toEqual({ mode: "full", scale: KPI_FIT_FLOOR })
    expect(fitKpi(50, 196, 196)).toEqual({ mode: "full", scale: KPI_FIT_FLOOR })
  })

  it("does not guess before anything is measured", () => {
    expect(fitKpi(0, 196, 90)).toEqual(KPI_FIT_NATURAL)
    expect(fitKpi(300, 0, null)).toEqual(KPI_FIT_NATURAL)
    expect(fitKpi(Number.NaN, 196, 90)).toEqual(KPI_FIT_NATURAL)
  })

  it("treats a change under a hundredth of the size as the same fit", () => {
    expect(sameKpiFit({ mode: "full", scale: 0.8 }, { mode: "full", scale: 0.805 })).toBe(true)
    expect(sameKpiFit({ mode: "full", scale: 0.8 }, { mode: "full", scale: 0.82 })).toBe(false)
    expect(sameKpiFit({ mode: "full", scale: 0.6 }, { mode: "compact", scale: 0.6 })).toBe(false)
  })
})

describe("compactKpiText: the value's short form under its own format", () => {
  it("writes K, M, B with one decimal at most, in the number locale", () => {
    expect(compactKpiText(13_330_172, undefined, "en-US")).toBe("13.3M")
    expect(compactKpiText(13_330_172, { style: "number", decimals: 2 }, "en-US")).toBe("13.3 M")
    expect(compactKpiText(13_330_172, { style: "number" }, "id-ID")).toBe("13,3 M")
  })

  it("keeps the currency, the prefix and the suffix", () => {
    expect(compactKpiText(1_234_567, { style: "currency", currency: "IDR" }, "id-ID")).toBe("Rp 1,2 M")
    expect(compactKpiText(13_330_172, { style: "number", suffix: " visits" }, "en-US")).toBe("13.3 M visits")
  })

  it("has none below 1,000 or for a style that is already short", () => {
    expect(compactKpiText(999, undefined)).toBeNull()
    expect(compactKpiText(0.125, { style: "percent" })).toBeNull()
    expect(compactKpiText(13_330_172, { style: "scientific" })).toBeNull()
    expect(compactKpiText(13_330_172, { style: "compact" })).toBeNull()
    expect(compactKpiText(Number.POSITIVE_INFINITY, undefined)).toBeNull()
  })
})
