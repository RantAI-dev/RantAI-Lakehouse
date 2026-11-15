import { describe, expect, it } from "bun:test"
import type { LayoutMap } from "@/services"
import { MIN_TILE_WIDTH, REFERENCE_CANVAS, reflowNarrow, tileWidthPx } from "./grid-reflow"

// Owner QA 2026-11-15: a canvas narrowed by a side panel lays a squeezed row out again instead of
// shrinking its tiles under MIN_TILE_WIDTH; the saved layout is never touched. BI-61 review,
// reflow SHOULD-FIX 1 (only tiles squeezed by the space, not small by design) and SHOULD-FIX 2
// (never narrower, balanced rows).
const row = (ws: number[], h = 3, y = 0): LayoutMap => {
  const out: LayoutMap = {}
  let x = 0
  ws.forEach((w, i) => { out[`t${i + 1}`] = { x, y, w, h }; x += w })
  return out
}

/** Every tile at least as wide as authored (SHOULD-FIX 2). */
function neverNarrower(before: LayoutMap, after: LayoutMap) {
  for (const [id, b] of Object.entries(before)) expect(after[id].w).toBeGreaterThanOrEqual(b.w)
}

describe("reflowNarrow", () => {
  it("measures a tile as its columns and the gaps between them", () => {
    // 900 px canvas: (900 - 11 * 12) / 12 = 64 px a column; three columns and two gaps.
    expect(tileWidthPx(3, 64, 12)).toBe(216)
    expect(MIN_TILE_WIDTH).toBe(240)
    expect(REFERENCE_CANVAS).toBe(1200)
  })

  it("keeps the saved boxes when every tile is wide enough", () => {
    const kpis = row([3, 3, 3, 3])
    // 1350 px: a three-column tile is about 328 px.
    expect(reflowNarrow(kpis, 1350)).toBe(kpis)
  })

  // SHOULD-FIX 1: a 2-column tile is 190 px on the 1200 px reference, so it was made small on purpose.
  it("leaves six 2-column KPIs as authored on a 1440 px laptop, and on narrower canvases too", () => {
    const six = row([2, 2, 2, 2, 2, 2])
    expect(reflowNarrow(six, 1440)).toBe(six)
    expect(reflowNarrow(six, 1350)).toBe(six)
    expect(reflowNarrow(six, 900)).toBe(six)
  })

  it("leaves a band alone when its only narrow tiles are small by design", () => {
    const layout = row([2, 10])
    expect(reflowNarrow(layout, 900)).toBe(layout)
  })

  it("lays four squeezed 3-column KPIs out 2 + 2 at 900 px, and moves what is below down by what it grew", () => {
    const layout: LayoutMap = { ...row([3, 3, 3, 3]), chart: { x: 0, y: 4, w: 12, h: 6, tab: "t1" } }
    const out = reflowNarrow(layout, 900)
    expect(out.t1).toEqual({ x: 0, y: 0, w: 6, h: 3 })
    expect(out.t2).toEqual({ x: 6, y: 0, w: 6, h: 3 })
    expect(out.t3).toEqual({ x: 0, y: 3, w: 6, h: 3 })
    expect(out.t4).toEqual({ x: 6, y: 3, w: 6, h: 3 })
    // The band was 3 rows and is now 6; the one-row gap the author left above the chart stays.
    expect(out.chart).toEqual({ x: 0, y: 7, w: 12, h: 6, tab: "t1" })
  })

  it("never shrinks the chart beside a squeezed KPI: [3, 9] at 900 px puts each on its own row", () => {
    const layout: LayoutMap = {
      kpi: { x: 0, y: 0, w: 3, h: 3 },
      chart: { x: 3, y: 0, w: 9, h: 6 },
      below: { x: 0, y: 6, w: 12, h: 2 },
    }
    const out = reflowNarrow(layout, 900)
    // A KPI alone keeps half the canvas; the band's main tile (wider than every other) takes all of it.
    expect(out.kpi).toEqual({ x: 0, y: 0, w: 6, h: 3 })
    expect(out.chart).toEqual({ x: 0, y: 3, w: 12, h: 6 })
    // The band grew from 6 rows to 9.
    expect(out.below).toEqual({ x: 0, y: 9, w: 12, h: 2 })
    neverNarrower(layout, out)
  })

  it("keeps a chart that spans two KPI rows at least as wide, with the KPIs spread over the rows", () => {
    const layout: LayoutMap = {
      chart: { x: 0, y: 0, w: 6, h: 8 },
      k1: { x: 6, y: 0, w: 3, h: 4 },
      k2: { x: 9, y: 0, w: 3, h: 4 },
      k3: { x: 6, y: 4, w: 3, h: 4 },
      k4: { x: 9, y: 4, w: 3, h: 4 },
    }
    const out = reflowNarrow(layout, 900)
    // Needs 6 + 4 + 4 + 4 + 4 columns: 2 + 3 is the only even split that fits two rows. The chart's
    // row shares its 2 spare columns by the authored widths (6 : 3).
    expect(out.chart).toEqual({ x: 0, y: 0, w: 7, h: 8 })
    expect(out.k1).toEqual({ x: 7, y: 0, w: 5, h: 4 })
    expect(out.k2).toEqual({ x: 0, y: 8, w: 4, h: 4 })
    expect(out.k3).toEqual({ x: 4, y: 8, w: 4, h: 4 })
    expect(out.k4).toEqual({ x: 8, y: 8, w: 4, h: 4 })
    neverNarrower(layout, out)
  })

  it("spreads five tiles (3, 3, 2, 2, 2) over 3 + 2 at 900 px, each at least the minimum", () => {
    const layout = row([3, 3, 2, 2, 2])
    const out = reflowNarrow(layout, 900)
    expect(out.t1).toEqual({ x: 0, y: 0, w: 4, h: 3 })
    expect(out.t2).toEqual({ x: 4, y: 0, w: 4, h: 3 })
    expect(out.t3).toEqual({ x: 8, y: 0, w: 4, h: 3 })
    expect(out.t4).toEqual({ x: 0, y: 3, w: 6, h: 3 })
    expect(out.t5).toEqual({ x: 6, y: 3, w: 6, h: 3 })
    // Four columns at 900 px is 292 px.
    for (const b of Object.values(out)) expect(tileWidthPx(b.w, 64, 12)).toBeGreaterThanOrEqual(MIN_TILE_WIDTH)
    neverNarrower(layout, out)
  })

  it("spreads six tiles 3 + 3 when one row cannot hold them", () => {
    // 900 px: one squeezed 3-column tile triggers the band; each tile needs 4 columns (24 in all).
    const layout = row([3, 2, 2, 2, 2, 1])
    const out = reflowNarrow(layout, 900)
    expect(["t1", "t2", "t3"].map((id) => [out[id].x, out[id].y, out[id].w])).toEqual([[0, 0, 4], [4, 0, 4], [8, 0, 4]])
    expect(["t4", "t5", "t6"].map((id) => [out[id].x, out[id].y, out[id].w])).toEqual([[0, 3, 4], [4, 3, 4], [8, 3, 4]])
    neverNarrower(layout, out)
  })

  it("keeps three squeezed tiles on one row when they fit, and goes 2 + 1 only when they do not", () => {
    // 900 px: three 3-column tiles need 4 columns each, 12 in all: one row.
    const three = row([3, 3, 3])
    const wide = reflowNarrow(three, 900)
    expect([wide.t1, wide.t2, wide.t3].map((b) => [b.x, b.y, b.w])).toEqual([[0, 0, 4], [4, 0, 4], [8, 0, 4]])
    // 700 px: they need 5 columns each, 15 in all: 2 + 1, and the last keeps half the canvas.
    const narrow = reflowNarrow(three, 700)
    expect(narrow.t1).toEqual({ x: 0, y: 0, w: 6, h: 3 })
    expect(narrow.t2).toEqual({ x: 6, y: 0, w: 6, h: 3 })
    expect(narrow.t3).toEqual({ x: 0, y: 3, w: 6, h: 3 })
  })

  it("never changes the layout it is given", () => {
    const layout = row([3, 3, 3, 3])
    const before = JSON.stringify(layout)
    reflowNarrow(layout, 900)
    expect(JSON.stringify(layout)).toBe(before)
  })

  it("reflows only the bands that hold a squeezed tile", () => {
    const layout: LayoutMap = { wide1: { x: 0, y: 0, w: 6, h: 4 }, wide2: { x: 6, y: 0, w: 6, h: 4 }, ...row([3, 3, 3, 3], 3, 4) }
    const out = reflowNarrow(layout, 900)
    expect(out.wide1).toBe(layout.wide1)
    expect(out.wide2).toBe(layout.wide2)
    expect(out.t3).toEqual({ x: 0, y: 7, w: 6, h: 3 })
  })

  it("does nothing before the canvas is measured", () => {
    const kpis = row([3, 3, 3, 3])
    expect(reflowNarrow(kpis, 0)).toBe(kpis)
  })
})
