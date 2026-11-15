import { describe, expect, it } from "bun:test"
import type { LayoutMap } from "@/services"
import { MIN_TILE_WIDTH, reflowNarrow, tileWidthPx } from "./grid-reflow"

// Owner QA 2026-11-15: a canvas narrowed by a side panel draws a row two tiles a row instead of
// shrinking its tiles under MIN_TILE_WIDTH; the saved layout is never touched.
const kpis: LayoutMap = {
  k1: { x: 0, y: 0, w: 3, h: 3 },
  k2: { x: 3, y: 0, w: 3, h: 3 },
  k3: { x: 6, y: 0, w: 3, h: 3 },
  k4: { x: 9, y: 0, w: 3, h: 3 },
  chart: { x: 0, y: 4, w: 12, h: 6, tab: "t1" },
}

describe("reflowNarrow", () => {
  it("measures a tile as its columns and the gaps between them", () => {
    // 900 px canvas: (900 - 11 * 12) / 12 = 64 px a column; three columns and two gaps.
    expect(tileWidthPx(3, 64, 12)).toBe(216)
    expect(MIN_TILE_WIDTH).toBe(240)
  })

  it("keeps the saved boxes when every tile is wide enough", () => {
    // 1350 px: a three-column tile is about 328 px.
    expect(reflowNarrow(kpis, 1350)).toBe(kpis)
  })

  it("lays a row of narrow tiles out two a row, in reading order, and moves what is below down by what it grew", () => {
    // 900 px: a three-column tile would be 216 px.
    const out = reflowNarrow(kpis, 900)
    expect(out.k1).toEqual({ x: 0, y: 0, w: 6, h: 3 })
    expect(out.k2).toEqual({ x: 6, y: 0, w: 6, h: 3 })
    expect(out.k3).toEqual({ x: 0, y: 3, w: 6, h: 3 })
    expect(out.k4).toEqual({ x: 6, y: 3, w: 6, h: 3 })
    // The band was 3 rows and is now 6; the one-row gap the author left above the chart stays.
    expect(out.chart).toEqual({ x: 0, y: 7, w: 12, h: 6, tab: "t1" })
  })

  it("never changes the layout it is given", () => {
    const before = JSON.stringify(kpis)
    reflowNarrow(kpis, 900)
    expect(JSON.stringify(kpis)).toBe(before)
  })

  it("gives an odd last tile the full width, and a pair the height of its taller tile", () => {
    const layout: LayoutMap = {
      a: { x: 0, y: 0, w: 3, h: 4 },
      b: { x: 3, y: 0, w: 3, h: 6 },
      c: { x: 6, y: 0, w: 6, h: 3 },
      d: { x: 0, y: 6, w: 12, h: 2 },
    }
    const out = reflowNarrow(layout, 900)
    expect(out.a).toEqual({ x: 0, y: 0, w: 6, h: 4 })
    expect(out.b).toEqual({ x: 6, y: 0, w: 6, h: 6 })
    expect(out.c).toEqual({ x: 0, y: 6, w: 12, h: 3 })
    // The band grew from 6 rows to 9.
    expect(out.d).toEqual({ x: 0, y: 9, w: 12, h: 2 })
  })

  it("treats tiles that overlap in height as one row, even when they start lower", () => {
    const layout: LayoutMap = {
      tall: { x: 0, y: 0, w: 3, h: 8 },
      top: { x: 3, y: 0, w: 9, h: 4 },
      low: { x: 3, y: 4, w: 9, h: 4 },
    }
    const out = reflowNarrow(layout, 900)
    expect(out.tall).toEqual({ x: 0, y: 0, w: 6, h: 8 })
    expect(out.top).toEqual({ x: 6, y: 0, w: 6, h: 4 })
    expect(out.low).toEqual({ x: 0, y: 8, w: 12, h: 4 })
  })

  it("reflows only the rows that hold a narrow tile", () => {
    const layout: LayoutMap = {
      wide1: { x: 0, y: 0, w: 6, h: 4 },
      wide2: { x: 6, y: 0, w: 6, h: 4 },
      n1: { x: 0, y: 4, w: 2, h: 3 },
      n2: { x: 2, y: 4, w: 10, h: 3 },
    }
    const out = reflowNarrow(layout, 1100)
    expect(out.wide1).toBe(layout.wide1)
    expect(out.wide2).toBe(layout.wide2)
    expect(out.n1).toEqual({ x: 0, y: 4, w: 6, h: 3 })
    expect(out.n2).toEqual({ x: 6, y: 4, w: 6, h: 3 })
  })

  it("does nothing before the canvas is measured", () => {
    expect(reflowNarrow(kpis, 0)).toBe(kpis)
  })
})
