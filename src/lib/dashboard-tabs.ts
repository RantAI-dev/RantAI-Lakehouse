/**
 * Tabs, tab ties and the phone layout of a dashboard (BI-18 part C).
 *
 * Pure rules the page depends on, mirroring `lakehouse_bi::layout`: which tab
 * a tile sits on, which filters a tile is given, how a tab is added, moved or
 * deleted without leaving a tile or a filter on a tab that no longer exists,
 * and the order a phone shows its tiles in. No fetch and no React.
 */

import type { BoardTab, FilterDef, LayoutMap, PhoneTile, TileBox } from "@/services/clients/bi-store"
import type { ChartKind } from "./dashboard-specs"

/** Query parameter holding the selected tab. */
export const TAB_PARAM = "tab"
/** Same ceilings as the API (`MAX_TABS`, `TAB_TITLE_MAX_CHARS`). */
export const MAX_TABS = 20
export const TAB_TITLE_MAX = 60
/** Below this width the tiles stack in one column (px). */
export const PHONE_MAX_WIDTH = 640

const STATIC_KINDS: ReadonlySet<string> = new Set(["text", "heading", "divider", "section"])

/** A tile that carries no data: it has no statement and takes no filter. Mirrors `ChartKind::is_static`. */
export function isStaticKind(kind: ChartKind | string): boolean {
  return STATIC_KINDS.has(kind)
}

/** The tab a tile sits on: the one it names, else the first; `undefined` on a board with no tabs. */
export function tabOf(tabs: readonly BoardTab[], layout: LayoutMap, id: string): string | undefined {
  if (tabs.length === 0) return undefined
  const named = layout[id]?.tab
  return tabs.find((t) => t.id === named)?.id ?? tabs[0].id
}

/** Whether a filter is given to a tile on `tab`. A board with no tabs (`tab` undefined) ignores ties. */
export function appliesOnTab(f: FilterDef, tab: string | undefined): boolean {
  if (tab === undefined || !f.tabs || f.tabs.length === 0) return true
  return f.tabs.includes(tab)
}

/** The filters a tile on `tab` is given; the same array when none is tied. */
export function filtersOnTab(filters: FilterDef[], tab: string | undefined): FilterDef[] {
  return filters.some((f) => f.tabs?.length) ? filters.filter((f) => appliesOnTab(f, tab)) : filters
}

/** A fresh tab id (letters and digits, which the API accepts). */
export function newTabId(): string {
  return `t_${Math.random().toString(16).slice(2, 10).padEnd(8, "0")}`
}

function clipTitle(title: string): string {
  return title.trim().slice(0, TAB_TITLE_MAX)
}

export type TabEdit = { tabs: BoardTab[]; layout: LayoutMap; filters: FilterDef[] }

/** The y a tile moved onto `tab` starts at: below everything already there. */
function bottomOf(layout: LayoutMap, ids: string[]): number {
  return ids.reduce((y, id) => Math.max(y, (layout[id]?.y ?? 0) + (layout[id]?.h ?? 0)), 0)
}

/**
 * A new tab at the end. Adding the first tab puts every existing tile in it
 * (decision 1), so a board that had none keeps showing everything. Returns
 * null at the limit.
 */
export function addTab(edit: TabEdit, title: string, id: string = newTabId()): TabEdit | null {
  if (edit.tabs.length >= MAX_TABS) return null
  const tab = { id, title: clipTitle(title) || `Tab ${edit.tabs.length + 1}` }
  if (edit.tabs.length > 0) return { ...edit, tabs: [...edit.tabs, tab] }
  const layout: LayoutMap = {}
  for (const [key, box] of Object.entries(edit.layout)) layout[key] = { ...box, tab: id }
  return { ...edit, tabs: [tab], layout }
}

export function renameTab(tabs: BoardTab[], id: string, title: string): BoardTab[] {
  const clean = clipTitle(title)
  return clean ? tabs.map((t) => (t.id === id ? { ...t, title: clean } : t)) : tabs
}

/** Moves a tab one place left (-1) or right (1). */
export function moveTab(tabs: BoardTab[], id: string, by: -1 | 1): BoardTab[] {
  const i = tabs.findIndex((t) => t.id === id)
  const j = i + by
  if (i < 0 || j < 0 || j >= tabs.length) return tabs
  const next = [...tabs]
  ;[next[i], next[j]] = [next[j], next[i]]
  return next
}

/** The ids of the tiles on `tab`, given every id the page knows. */
export function tilesOnTab(tabs: readonly BoardTab[], layout: LayoutMap, ids: string[], tab: string): string[] {
  return ids.filter((id) => tabOf(tabs, layout, id) === tab)
}

/**
 * Deletes a tab. Its tiles go to the first remaining tab, below what is
 * there, or, when `moveTiles` is false, the tab must be empty. A filter tied
 * to it loses that tie; one left with no tie is tied to the tab its tiles
 * went to, so deleting a tab never makes a filter apply where it did not.
 * Deleting the last tab leaves a board with no tabs (and no ties).
 */
export function deleteTab(
  edit: TabEdit, ids: string[], id: string, moveTiles: boolean,
): TabEdit | { error: string } {
  const index = edit.tabs.findIndex((t) => t.id === id)
  if (index < 0) return edit
  const remaining = edit.tabs.filter((t) => t.id !== id)
  const own = tilesOnTab(edit.tabs, edit.layout, ids, id)
  if (own.length > 0 && !moveTiles) return { error: "Move the tiles out of this tab first, or choose to move them to the first tab." }
  if (remaining.length === 0) {
    const layout: LayoutMap = {}
    for (const [key, box] of Object.entries(edit.layout)) layout[key] = { x: box.x, y: box.y, w: box.w, h: box.h }
    return { tabs: [], layout, filters: edit.filters.map(untied) }
  }
  const target = remaining[0].id
  const layout: LayoutMap = { ...edit.layout }
  let y = bottomOf(edit.layout, tilesOnTab(edit.tabs, edit.layout, ids, target))
  for (const tile of own) {
    const box = edit.layout[tile] ?? { x: 0, y: 0, w: 6, h: 6 }
    layout[tile] = { ...box, tab: target, x: 0, y }
    y += box.h
  }
  // Tiles outside `ids` that name the deleted tab (a chart removed elsewhere) follow to the first tab too.
  for (const [key, box] of Object.entries(layout)) if (box.tab === id) layout[key] = { ...box, tab: target }
  const filters = edit.filters.map((f) => {
    if (!f.tabs?.includes(id)) return f
    const left = f.tabs.filter((t) => t !== id)
    return { ...f, tabs: left.length ? left : [target] }
  })
  return { tabs: remaining, layout, filters }
}

/** Puts a tile on another tab, below what is already there. */
export function moveTileToTab(tabs: readonly BoardTab[], layout: LayoutMap, ids: string[], id: string, tab: string): LayoutMap {
  const others = tilesOnTab(tabs, layout, ids, tab).filter((x) => x !== id)
  const box = layout[id] ?? { x: 0, y: 0, w: 6, h: 6 }
  return { ...layout, [id]: { ...box, tab, x: 0, y: bottomOf(layout, others) } }
}

/** The filter with no tie. */
function untied(f: FilterDef): FilterDef {
  const copy = { ...f }
  delete copy.tabs
  return copy
}

/** Ties a filter to the given tabs; all or none means every tab (no key). */
export function withTies(f: FilterDef, tabs: readonly BoardTab[], chosen: string[]): FilterDef {
  const rest = untied(f)
  const valid = chosen.filter((c) => tabs.some((t) => t.id === c))
  return valid.length === 0 || valid.length === tabs.length ? rest : { ...rest, tabs: valid }
}

/** Reading order: row, then column. */
export function readingOrder(ids: string[], layout: LayoutMap): string[] {
  const at = (id: string): TileBox => layout[id] ?? { x: 0, y: Number.MAX_SAFE_INTEGER, w: 0, h: 0 }
  return [...ids].sort((a, b) => at(a).y - at(b).y || at(a).x - at(b).x)
}

/**
 * The tiles of a phone, in order, with a height each. With a saved phone
 * layout its order wins and a tile it does not name (added since) follows in
 * reading order; without one the order is the reading order and the height
 * the tile's own.
 */
export function phoneOrder(ids: string[], layout: LayoutMap, phone: readonly PhoneTile[] | undefined): PhoneTile[] {
  const own = (id: string) => ({ id, h: layout[id]?.h ?? 6 })
  const reading = readingOrder(ids, layout)
  if (!phone || phone.length === 0) return reading.map(own)
  const known = new Set(ids)
  const named = phone.filter((p) => known.has(p.id))
  const seen = new Set(named.map((p) => p.id))
  return [...named, ...reading.filter((id) => !seen.has(id)).map(own)]
}

/** The phone layout to save after moving `id` one place up (-1) or down (1) in `order`. */
export function movePhoneTile(order: PhoneTile[], id: string, by: -1 | 1): PhoneTile[] {
  const i = order.findIndex((p) => p.id === id)
  const j = i + by
  if (i < 0 || j < 0 || j >= order.length) return order
  const next = [...order]
  ;[next[i], next[j]] = [next[j], next[i]]
  return next
}

export function resizePhoneTile(order: PhoneTile[], id: string, h: number): PhoneTile[] {
  const clamped = Math.min(60, Math.max(1, Math.round(h)))
  return order.map((p) => (p.id === id ? { ...p, h: clamped } : p))
}

/** `?tab=` from a query string: the id when it names a tab, else the first. */
export function selectedTab(tabs: readonly BoardTab[], param: string | null): string | undefined {
  if (tabs.length === 0) return undefined
  return tabs.find((t) => t.id === param)?.id ?? tabs[0].id
}

/** A query string with the selected tab set (the first tab is the default and leaves no parameter). */
export function withTabParam(search: string, tabs: readonly BoardTab[], tab: string): string {
  const q = new URLSearchParams(search)
  if (tabs[0]?.id === tab) q.delete(TAB_PARAM)
  else q.set(TAB_PARAM, tab)
  return q.toString()
}

/**
 * Folding (BI-18 part C): a `section` tile folds the tiles after it, in
 * reading order, up to the next section on the same tab. `ids` are the tiles
 * of one tab with their kind. Returns the hidden ids.
 */
export function foldedTiles(items: { id: string; kind: string }[], layout: LayoutMap, folded: ReadonlySet<string>): Set<string> {
  const hidden = new Set<string>()
  const kind = new Map(items.map((i) => [i.id, i.kind]))
  let folding = false
  for (const id of readingOrder(items.map((i) => i.id), layout)) {
    if (kind.get(id) === "section") { folding = folded.has(id); continue }
    if (folding) hidden.add(id)
  }
  return hidden
}

/**
 * The layout a folded dashboard is drawn with: the hidden tiles are gone and
 * the tiles below each folded group move up by the group's height.
 */
export function collapseLayout(layout: LayoutMap, items: { id: string; kind: string }[], hidden: ReadonlySet<string>): LayoutMap {
  const kind = new Map(items.map((i) => [i.id, i.kind]))
  const groups: { top: number; bottom: number }[] = []
  let current: string[] = []
  const flush = () => {
    if (current.length) {
      groups.push({
        top: Math.min(...current.map((id) => layout[id].y)),
        bottom: Math.max(...current.map((id) => layout[id].y + layout[id].h)),
      })
    }
    current = []
  }
  for (const id of readingOrder(items.map((i) => i.id), layout)) {
    if (kind.get(id) === "section") flush()
    else if (hidden.has(id) && layout[id]) current.push(id)
  }
  flush()
  const out: LayoutMap = {}
  for (const [id, box] of Object.entries(layout)) if (!hidden.has(id)) out[id] = { ...box }
  for (const g of [...groups].sort((a, b) => b.top - a.top)) {
    for (const box of Object.values(out)) if (box.y >= g.bottom) box.y -= g.bottom - g.top
  }
  return out
}
