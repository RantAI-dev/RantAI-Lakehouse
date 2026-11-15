import type { LayoutMap, TileBox } from "@/services"

/**
 * The narrowest a tile is drawn on the dashboard canvas when reading: 240 px
 * (owner QA 2026-11-15). Below it a tile's text stops reading well: at 240 px
 * a KPI has about 208 px for its number (tile and body padding off), which
 * holds "13,330,172" at its full size, and a header keeps about 180 px of
 * title beside the ⋯; a chart keeps room for an axis and a few bars.
 */
export const MIN_TILE_WIDTH = 240

/**
 * The canvas a layout is judged on to tell a tile made small on purpose from
 * one the space has squeezed: 1200 px, the narrowest common laptop content
 * width (a 1280 px window less the sidebar and page padding). BI-61 review,
 * reflow SHOULD-FIX 1: a tile already under `MIN_TILE_WIDTH` here (a 2-column
 * tile is 190 px) was authored small, so it stays as authored on any screen
 * and relies on the KPI fit and the cut-text tooltips instead; only a tile
 * that is wide enough here and under the minimum now asks for a reflow.
 */
export const REFERENCE_CANVAS = 1200

/** The drawn width of a box `w` columns wide: its columns and the gaps between them. */
export function tileWidthPx(w: number, colW: number, margin: number): number {
  return w * colW + (w - 1) * margin
}

const columnWidth = (width: number, cols: number, margin: number) => (width - margin * (cols - 1)) / cols

/** Whether `a` puts more tiles in an earlier row than `b` (numeric, row by row). */
function fullerFirst(a: readonly number[], b: readonly number[]): boolean {
  for (let i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) return a[i] > b[i]
  return false
}

/**
 * The contiguous splits of `sizes` (in order) into `rows` rows, each row's
 * sum at most `cols`; the most even in tile count first, the fuller rows
 * first on a tie. Null when none fits. Bands hold a handful of tiles, so the
 * splits are simply enumerated.
 */
function balancedSplit(sizes: readonly number[], rows: number, cols: number): number[] | null {
  let best: number[] | null = null
  let bestSpread = Infinity
  const walk = (start: number, left: number, counts: number[]) => {
    if (left === 1) {
      const rest = sizes.length - start
      if (rest < 1 || sizes.slice(start).reduce((a, b) => a + b, 0) > cols) return
      const all = [...counts, rest]
      const spread = Math.max(...all) - Math.min(...all)
      // Equal spread: the split whose first rows are fuller (3 + 2, not 2 + 3) reads better.
      if (spread < bestSpread || (spread === bestSpread && best && fullerFirst(all, best))) {
        best = all
        bestSpread = spread
      }
      return
    }
    let sum = 0
    for (let n = 1; start + n <= sizes.length - (left - 1); n++) {
      sum += sizes[start + n - 1]
      if (sum > cols) break
      walk(start + n, left - 1, [...counts, n])
    }
  }
  walk(0, rows, [])
  return best
}

/**
 * Spreads `cols` columns over a row: each tile starts at its `need` and the
 * columns left over go in proportion to the authored widths `w` (largest
 * remainder), so the row fills the canvas and keeps the author's proportions.
 */
function fillRow(need: readonly number[], w: readonly number[], cols: number): number[] {
  const out = [...need]
  const extra = cols - need.reduce((a, b) => a + b, 0)
  const total = w.reduce((a, b) => a + b, 0)
  if (extra <= 0 || total <= 0) return out
  const shares = w.map((x) => (extra * x) / total)
  shares.forEach((s, i) => { out[i] += Math.floor(s) })
  let left = extra - shares.reduce((a, s) => a + Math.floor(s), 0)
  const order = shares.map((s, i) => [s - Math.floor(s), i] as const).sort((a, b) => b[0] - a[0] || a[1] - b[1])
  for (const [, i] of order) {
    if (left <= 0) break
    out[i] += 1
    left -= 1
  }
  return out
}

/**
 * The layout a reading viewer sees on a canvas `width` px wide (owner QA
 * 2026-11-15; rule rewritten by BI-61 review, reflow SHOULD-FIX 1 and 2).
 *
 * - A "row" is a band: the tiles whose vertical extents overlap, directly or
 *   through each other, since the canvas stores free boxes, not rows.
 * - A band is laid out again only when it holds a squeezed tile: under
 *   `MIN_TILE_WIDTH` now and at least that on `REFERENCE_CANVAS`. A band whose
 *   narrow tiles are all small by design keeps its boxes.
 * - In a laid-out band every tile needs at least its authored width and at
 *   least the minimum, in whole columns, so no tile ever gets narrower. The
 *   tiles keep reading order and go into the fewest rows that hold them,
 *   spread evenly (4 → 2 + 2, 5 → 3 + 2, 6 → 3 + 3). A row of two or more
 *   fills the canvas in proportion to the authored widths.
 * - A tile alone in its row takes half the canvas (or its need, if more),
 *   and the full width only when it is the band's main tile, wider than
 *   every other tile in it (a chart beside KPIs), never a stretched KPI.
 * - A row is as tall as its tallest tile; each tile keeps its own height.
 *   Every band below moves down by what the band grew, so the author's gaps
 *   between bands stay.
 *
 * View only: the saved layout is never changed, and edit mode draws the real
 * grid so its author edits what is saved.
 */
export function reflowNarrow(
  layout: LayoutMap,
  width: number,
  { cols = 12, margin = 12, minWidth = MIN_TILE_WIDTH, reference = REFERENCE_CANVAS }:
    { cols?: number; margin?: number; minWidth?: number; reference?: number } = {},
): LayoutMap {
  if (!(width > 0)) return layout
  const colW = columnWidth(width, cols, margin)
  const refColW = columnWidth(reference, cols, margin)
  // SHOULD-FIX 1: squeezed by the space, not small by the author's choice.
  const squeezed = (w: number) => tileWidthPx(w, colW, margin) < minWidth && tileWidthPx(w, refColW, margin) >= minWidth
  // The fewest whole columns at least `minWidth` wide on this canvas.
  const minCols = Math.min(cols, Math.max(1, Math.ceil((minWidth + margin) / (colW + margin))))

  const ids = Object.keys(layout).sort((a, b) => layout[a].y - layout[b].y || layout[a].x - layout[b].x)
  const bands: string[][] = []
  let bottom = -Infinity
  for (const id of ids) {
    const b = layout[id]
    if (bands.length && b.y < bottom) {
      bands[bands.length - 1].push(id)
      bottom = Math.max(bottom, b.y + b.h)
    } else {
      bands.push([id])
      bottom = b.y + b.h
    }
  }

  const half = Math.ceil(cols / 2)
  const out: LayoutMap = {}
  let shift = 0
  let changed = false
  for (const band of bands) {
    const top = Math.min(...band.map((id) => layout[id].y))
    const end = Math.max(...band.map((id) => layout[id].y + layout[id].h))
    if (!band.some((id) => squeezed(layout[id].w))) {
      for (const id of band) out[id] = shift ? { ...layout[id], y: layout[id].y + shift } : layout[id]
      continue
    }
    changed = true
    // SHOULD-FIX 2: never narrower than authored, never under the minimum.
    const need = band.map((id) => Math.min(cols, Math.max(layout[id].w, minCols)))
    let counts: number[] | null = null
    for (let rows = 1; rows <= band.length && !counts; rows++) counts = balancedSplit(need, rows, cols)
    const split = counts ?? band.map(() => 1)
    // The band's main tile: wider than every other one (three equal KPIs have none).
    const widths0 = band.map((id) => layout[id].w)
    const widest = Math.max(...widths0)
    const main = widths0.filter((w) => w === widest).length === 1 ? widest : Infinity
    let y = top + shift
    let at = 0
    for (const n of split) {
      const row = band.slice(at, at + n)
      const rowNeed = need.slice(at, at + n)
      at += n
      const widths = n > 1
        ? fillRow(rowNeed, row.map((id) => layout[id].w), cols)
        : [layout[row[0]].w === main && band.length > 1 ? cols : Math.max(rowNeed[0], half)]
      let x = 0
      row.forEach((id, j) => {
        const box: TileBox = layout[id]
        out[id] = { ...box, x, y, w: widths[j] }
        x += widths[j]
      })
      y += Math.max(...row.map((id) => layout[id].h))
    }
    shift += y - (top + shift) - (end - top)
  }
  return changed ? out : layout
}
