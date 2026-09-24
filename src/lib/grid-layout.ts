import type { LayoutMap, TileBox } from "@/services/clients/bi-store"

function overlaps(a: TileBox, b: TileBox): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/**
 * Pushes overlapping tiles down until none overlap.
 *
 * `anchorId` — the tile being dragged or resized — keeps its box and wins
 * every collision; the rest are settled top to bottom, left to right, each
 * dropping just below whatever it hits. Nothing is pulled up, so gaps the
 * user left stay where they are.
 */
export function settleLayout(layout: LayoutMap, anchorId?: string): LayoutMap {
  const order = Object.keys(layout).sort((a, b) => {
    if (a === anchorId) return -1
    if (b === anchorId) return 1
    const A = layout[a]
    const B = layout[b]
    return A.y - B.y || A.x - B.x
  })
  const placed: TileBox[] = []
  const out: LayoutMap = {}
  for (const id of order) {
    const box = { ...layout[id] }
    let hit = placed.find((p) => overlaps(box, p))
    while (hit) {
      box.y = hit.y + hit.h
      hit = placed.find((p) => overlaps(box, p))
    }
    placed.push(box)
    out[id] = box
  }
  return out
}
