import type { Hierarchy } from "@/services/contracts/hierarchies"
import { MAX_LEVELS, MIN_LEVELS } from "@/services/contracts/hierarchies"

/**
 * Hierarchies and drilling (`BI-10`): the pure rules the editor and the click
 * menu use, mirroring `lakehouse_bi::hierarchies` and `routes::drill`. The
 * server decides again on every request.
 */

/** What is still needed before a hierarchy can be saved, in the editor's words. */
export function hierarchyProblems(name: string, levels: readonly string[]): string[] {
  const out: string[] = []
  if (!name.trim()) out.push("a name")
  const picked = levels.filter(Boolean)
  if (picked.length < MIN_LEVELS || levels.some((l) => !l)) out.push(`${MIN_LEVELS} to ${MAX_LEVELS} columns`)
  if (new Set(picked).size !== picked.length) out.push("each column once")
  return out
}

/** The step of a drill: the level the chart is on, the next one, and the hierarchy they belong to. */
export type DrillStep = { hierarchy: Hierarchy; from: string; to: string }

/**
 * The next level when `column` is a level of one of the source's hierarchies
 * and not its last, or the plain reason in the click menu's words. The first
 * hierarchy that holds the column wins.
 */
export function nextLevel(hierarchies: readonly Hierarchy[], column: string): DrillStep | { reason: string } {
  const holding = hierarchies.filter((h) => h.levels.includes(column))
  if (!holding.length) return { reason: "this column is not a level of a hierarchy" }
  for (const h of holding) {
    const at = h.levels.indexOf(column)
    if (at < h.levels.length - 1) return { hierarchy: h, from: column, to: h.levels[at + 1] }
  }
  return { reason: "this is the last level of its hierarchy" }
}

/** One step down: the value that was clicked in the level before. */
export type Drilled = { value: string; label?: string }

/** The column a chart shows after `steps` drills down `hierarchy` from `dimension`; null past the last level. */
export function levelAfter(hierarchy: Hierarchy, dimension: string, steps: number): string | null {
  const start = hierarchy.levels.indexOf(dimension)
  return start < 0 ? null : hierarchy.levels[start + steps] ?? null
}

/** Whether the chart, after `steps` drills, can go one level further. */
export function canDrillFurther(hierarchy: Hierarchy, dimension: string, steps: number): boolean {
  return levelAfter(hierarchy, dimension, steps + 1) !== null
}

/** The trail of a drill: "All", then the value clicked at each step. */
export function trailOf(steps: readonly Drilled[]): string[] {
  return ["All", ...steps.map((s) => s.label ?? s.value)]
}

/** The chart after climbing back to trail entry `index` (0 is the whole chart). */
export function climbTo(steps: readonly Drilled[], index: number): Drilled[] {
  return steps.slice(0, Math.max(0, index))
}

/**
 * The drill request for the preview route: the saved chart, the level to go
 * to and the values clicked so far, each paired with its column (the chart's
 * own dimension, then each level below it). The server builds the
 * definition; this only names it.
 */
export function drillRequest(
  chartId: string,
  hierarchy: Hierarchy,
  dimension: string,
  steps: readonly Drilled[],
  filters: readonly unknown[],
): { drillOf: string; to: string; fix: { column: string; value: string }[]; filters: unknown[] } | null {
  const start = hierarchy.levels.indexOf(dimension)
  const to = hierarchy.levels[start + steps.length]
  if (start < 0 || !steps.length || to === undefined) return null
  const fix = steps.map((s, i) => ({ column: hierarchy.levels[start + i], value: s.value }))
  return { drillOf: chartId, to, fix, filters: [...filters] }
}

/**
 * Why a saved chart cannot be drilled down, in the click menu's words, or null
 * when it can. The same rules as `routes::drill::drillable`; the server
 * decides again on every request.
 */
export function drillBlockReason(def: {
  kind?: string; dimension?: string; grain?: string; bin?: unknown; breakdownBin?: unknown
  steps?: unknown[]; other?: boolean; groupings?: unknown[]; layers?: unknown[]
} | undefined): string | null {
  if (!def) return "this chart has no definition to drill."
  if (!["bar", "hbar", "line", "area", "stacked", "pie", "rose", "funnel", "treemap"].includes(def.kind ?? "")) return "this kind of chart cannot be drilled down."
  if (def.grain) return "a chart grouped by a date cannot be drilled down."
  if (def.bin || def.breakdownBin) return "a chart grouped in bins cannot be drilled down."
  if (def.steps?.length) return "a chart with steps cannot be drilled down."
  if (def.other) return "a chart with an 'Other' group cannot be drilled down."
  if (def.groupings?.length) return "a table with several groupings cannot be drilled down."
  // BI-13: the other questions are matched on this dimension's values, which a drill changes.
  if (def.layers?.length) return "a chart that combines several questions cannot be drilled down."
  if (!def.dimension) return "this chart has no dimension to drill."
  return null
}

/** The equality filters a drill fixes, for a list of records that must agree with the drilled tile. */
export function fixedFilters(hierarchy: Hierarchy, dimension: string, steps: readonly Drilled[]): { column: string; values: string[] }[] {
  const start = hierarchy.levels.indexOf(dimension)
  if (start < 0) return []
  return steps.map((s, i) => ({ column: hierarchy.levels[start + i], values: [s.value] }))
}
