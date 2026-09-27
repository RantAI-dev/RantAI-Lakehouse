/**
 * Row → series shapes for the chart kinds whose data is not one value per
 * category: sankey, sunburst, boxplot and calendar. Pure, so they are
 * tested here and `features/dashboards/chart-option.ts` only styles them.
 *
 * Rows come from the API as the builder's SQL returns them: sankey and
 * sunburst use the breakdown shape (`x`, `series`, value), a boxplot one
 * row per category with a five-number array (`quantilesExact(0, .25, .5,
 * .75, 1)`), a calendar one row per day.
 */

type Row = Record<string, unknown>
const str = (v: unknown) => String(v ?? "")
const num = (v: unknown) => Number(v ?? 0)

/**
 * Sankey nodes must be unique and the graph acyclic. A value can appear on
 * both sides (e.g. "Other" as a region and as a channel), so node ids carry
 * their side; `label` is what is shown.
 */
export type SankeyNode = { name: string; label: string }
export type SankeyLink = { source: string; target: string; value: number }

export function toSankey(
  rows: Row[],
  x: string,
  series: string,
  y: string
): { nodes: SankeyNode[]; links: SankeyLink[] } {
  const nodes = new Map<string, SankeyNode>()
  const links = new Map<string, SankeyLink>()
  for (const r of rows) {
    const value = num(r[y])
    if (!(value > 0)) continue // a sankey link needs a positive width
    const from = `a:${str(r[x])}`
    const to = `b:${str(r[series])}`
    nodes.set(from, { name: from, label: str(r[x]) })
    nodes.set(to, { name: to, label: str(r[series]) })
    const key = `${from}\u0000${to}`
    const link = links.get(key)
    if (link) link.value += value
    else links.set(key, { source: from, target: to, value })
  }
  return { nodes: [...nodes.values()], links: [...links.values()] }
}

export type SunburstNode = { name: string; value?: number; children?: SunburstNode[] }

/** Two rings: `x` values inside, their `series` values outside. */
export function toSunburst(rows: Row[], x: string, series: string, y: string): SunburstNode[] {
  const parents = new Map<string, SunburstNode>()
  for (const r of rows) {
    const value = num(r[y])
    if (!(value > 0)) continue
    const parentName = str(r[x])
    let parent = parents.get(parentName)
    if (!parent) {
      parent = { name: parentName, children: [] }
      parents.set(parentName, parent)
    }
    parent.children!.push({ name: str(r[series]), value })
  }
  return [...parents.values()]
}

/**
 * Boxplot data: categories and `[min, q1, median, q3, max]` per category.
 * A row whose value is not a five-number array is dropped rather than
 * drawn wrong.
 */
export function toBoxplot(
  rows: Row[],
  x: string,
  y: string
): { categories: string[]; data: number[][] } {
  const categories: string[] = []
  const data: number[][] = []
  for (const r of rows) {
    const v = r[y]
    if (!Array.isArray(v) || v.length !== 5) continue
    const five = v.map(Number)
    if (five.some((n) => !Number.isFinite(n))) continue
    categories.push(str(r[x]))
    data.push(five)
  }
  return { categories, data }
}

/**
 * Calendar data: `[yyyy-mm-dd, value]` per day plus the range to draw — at
 * most the last 365 days of the data, since a multi-year calendar squeezes
 * every cell to a sliver. Values that are not a date (the builder cannot
 * type-check the column) are dropped; `range` is null when nothing usable
 * is left.
 */
export function toCalendar(
  rows: Row[],
  x: string,
  y: string
): { data: [string, number][]; range: [string, string] | null } {
  const data: [string, number][] = []
  for (const r of rows) {
    const day = str(r[x]).slice(0, 10)
    if (!/^\d{4}-\d{2}-\d{2}$/.test(day)) continue
    data.push([day, num(r[y])])
  }
  if (!data.length) return { data, range: null }
  const days = data.map((d) => d[0]).sort()
  const last = days[days.length - 1]
  const yearBack = new Date(`${last}T00:00:00Z`)
  yearBack.setUTCDate(yearBack.getUTCDate() - 364)
  const floor = yearBack.toISOString().slice(0, 10)
  return { data, range: [days[0] > floor ? days[0] : floor, last] }
}
