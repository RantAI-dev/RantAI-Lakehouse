import type { LayoutMap, TileBox } from "@/services"

/**
 * The narrowest a tile is drawn on the dashboard canvas when reading: 240 px
 * (owner QA 2026-11-15). Below it a tile's text stops reading well: at 240 px
 * a KPI has about 208 px for its number (tile and body padding off), which
 * holds "13,330,172" at its full size, and a header keeps about 180 px of
 * title beside the ⋯; a chart keeps room for an axis and a few bars.
 */
export const MIN_TILE_WIDTH = 240

/** The drawn width of a box `w` columns wide: its columns and the gaps between them. */
export function tileWidthPx(w: number, colW: number, margin: number): number {
  return w * colW + (w - 1) * margin
}

/**
 * The layout a reading viewer sees on a canvas `width` px wide (owner QA
 * 2026-11-15). The saved layout scales its 12 columns to the canvas; when that
 * would draw a tile narrower than `MIN_TILE_WIDTH`, the row it sits in is laid
 * out two tiles a row (half width each, in reading order) instead of
 * shrinking further. An odd last tile takes the full width, so no row is left
 * half empty.
 *
 * A "row" is a band: the tiles whose vertical extents overlap, directly or
 * through each other, since the canvas stores free boxes, not rows. A
 * reflowed band's rows are as tall as the taller tile of the pair; every band
 * below moves down by what the band grew, so the gaps the author left between
 * bands stay. Bands with no narrow tile keep their boxes.
 *
 * View only: the saved layout is never changed, and edit mode draws the real
 * grid so its author edits what is saved. Under the phone width the column
 * stack takes over, and there half the canvas is always at least 314 px, so a
 * half-width tile is never under the minimum itself.
 */
export function reflowNarrow(
  layout: LayoutMap,
  width: number,
  { cols = 12, margin = 12, minWidth = MIN_TILE_WIDTH }: { cols?: number; margin?: number; minWidth?: number } = {},
): LayoutMap {
  if (!(width > 0)) return layout
  const colW = (width - margin * (cols - 1)) / cols
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
  const half = Math.floor(cols / 2)
  const out: LayoutMap = {}
  let shift = 0
  let changed = false
  for (const band of bands) {
    const top = Math.min(...band.map((id) => layout[id].y))
    const end = Math.max(...band.map((id) => layout[id].y + layout[id].h))
    const narrow = band.some((id) => tileWidthPx(layout[id].w, colW, margin) < minWidth)
    if (!narrow) {
      for (const id of band) out[id] = shift ? { ...layout[id], y: layout[id].y + shift } : layout[id]
      continue
    }
    changed = true
    let y = top + shift
    for (let i = 0; i < band.length; i += 2) {
      const pair = band.slice(i, i + 2)
      const rowH = Math.max(...pair.map((id) => layout[id].h))
      pair.forEach((id, j) => {
        const box: TileBox = layout[id]
        out[id] = pair.length === 1
          ? { ...box, x: 0, y, w: cols }
          : { ...box, x: j * half, y, w: j === 0 ? half : cols - half }
      })
      y += rowH
    }
    shift += y - (top + shift) - (end - top)
  }
  return changed ? out : layout
}
