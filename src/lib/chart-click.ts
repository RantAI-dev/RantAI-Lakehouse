/**
 * What a click on a chart means (BI-18 part B): which column and which stored
 * value the clicked mark stands for. The ECharts event is read by the chart
 * wrapper; this module only maps it, so every kind's rule is testable
 * without a browser.
 *
 * Each kind reports differently, checked against ECharts 6.1 in a headless
 * run (a `map` series reports the feature name and the summed value, a
 * scatter on `geo` its item name, a calendar heatmap `name: ""` with
 * `value: [day, v]`, a sankey `dataType` "node"/"edge", a sunburst
 * `treePathInfo`, a boxplot the axis label with its "(n=…)" suffix). A drag
 * that pans a map is not a click: zrender drops a click whose mouse-up is
 * more than 4px from its mouse-down.
 */

import { toBoxplot } from "@/lib/chart-transforms"
import type { ChartKind, ChartSource } from "@/lib/dashboard-specs"

type Row = Record<string, unknown>

/** The parts of an ECharts click event the mapping reads. */
export type ChartHit = {
  name: string
  value?: unknown
  dataIndex?: number
  seriesType?: string
  /** Sankey: "node" or "edge". */
  dataType?: string
  /** Sunburst: the path from the root; its length less one is the ring. */
  treePathInfo?: { name: string }[]
}

/** What the page knows about the chart that was clicked. */
export type ClickSpec = {
  kind: ChartKind
  source: ChartSource
  /** The chart's category column; undefined when the chart has none (a stored chart without a definition). */
  dimension?: string
  /** The second dimension (sankey, sunburst). */
  breakdown?: string
  /** The measure columns (the boxplot reads its first). */
  y: string | string[]
}

/** What only the tile that was clicked can supply. */
export type ClickContext = {
  /** The rows the tile drew (a boxplot's categories are read from them). */
  rows?: Row[]
  /** Map feature → the distinct stored spellings joined to it (a choropleth). */
  regionNames?: ReadonlyMap<string, readonly string[]>
}

export type ChartClickHandler = (hit: ChartHit, pos: { x: number; y: number }, ctx: ClickContext) => void

export type DrillValue = { column: string; value: string }

/**
 * Kinds whose marks are not one value of a column. A geo heat map is drawn
 * as an image (ECharts reports no click on it), and a table, a KPI and a
 * gauge are one number or a list, not a mark per value: those offer the
 * rows behind the whole tile instead ({@link offersTileRecords}).
 */
const NO_VALUE_KINDS: ReadonlySet<ChartKind> = new Set(["geoheat", "table", "kpi", "gauge", "text"])

/**
 * Whether a click on this chart can ever name a value. A built-in line or
 * area is the exception among the category kinds: its axis is a derived
 * period (a year or a month computed by the spec's SQL), not a stored column
 * value, so there is nothing to drill into and it keeps having none.
 */
export function canDrill(spec: ClickSpec): boolean {
  if (NO_VALUE_KINDS.has(spec.kind)) return false
  if (spec.source === "builtin" && (spec.kind === "line" || spec.kind === "area")) return false
  return Boolean(spec.dimension)
}

/** Whether a chart of this kind has marks a click can name (the builder offers a click setting for it). */
export function kindHasClickValue(kind: ChartKind): boolean {
  return !NO_VALUE_KINDS.has(kind)
}

/** Whether the tile's menu offers "View records" for the whole tile. */
export function offersTileRecords(kind: ChartKind): boolean {
  return kind === "kpi" || kind === "gauge" || kind === "table" || kind === "geoheat"
}

const firstY = (y: string | string[]) => (Array.isArray(y) ? y[0] ?? "" : y)

/** `a:X` / `b:X` → the side and the value (a sankey node carries its side). */
function sankeyNode(name: string): { side: "a" | "b"; value: string } | null {
  const side = name.slice(0, 2)
  if (side !== "a:" && side !== "b:") return null
  return { side: side[0] as "a" | "b", value: name.slice(2) }
}

/**
 * The column and stored value a click stands for, or null when it stands for
 * none (no value drill: the click does nothing, rather than guessing).
 */
export function drillTarget(spec: ClickSpec, hit: ChartHit, ctx: ClickContext = {}): DrillValue | null {
  if (!canDrill(spec)) return null
  const column = spec.dimension ?? ""
  const named = (value: string, col = column): DrillValue | null => (value !== "" && col !== "" ? { column: col, value } : null)

  switch (spec.kind) {
    case "geomap": {
      // The stored spelling, not the map's: "Kab. Bandung" is what the
      // column holds, "Bandung" is what the feature is called. Two
      // spellings of one region have no single value to list.
      const spellings = ctx.regionNames?.get(hit.name)
      return spellings?.length === 1 ? named(spellings[0]) : null
    }
    case "pointmap":
      // The label column names a point; without one the coordinates are the
      // only name and they are not a column value.
      return named(hit.name)
    case "calendar": {
      // A calendar heatmap has no name; its value is [day, v].
      const day = Array.isArray(hit.value) ? hit.value[0] : undefined
      return typeof day === "string" ? named(day) : null
    }
    case "sankey": {
      if (hit.dataType !== "node") return null
      const node = sankeyNode(hit.name)
      if (!node) return null
      return node.side === "a" ? named(node.value) : named(node.value, spec.breakdown ?? "")
    }
    case "sunburst": {
      // Ring 1 is the dimension, ring 2 the breakdown; the root has no name.
      const ring = (hit.treePathInfo?.length ?? 0) - 1
      if (ring === 1) return named(hit.name)
      if (ring === 2) return named(hit.name, spec.breakdown ?? "")
      return null
    }
    case "boxplot": {
      // The axis label reads "category (n=3)"; the category is the row's.
      const categories = toBoxplot(ctx.rows ?? [], column, firstY(spec.y)).categories
      // The box carries its index as `dataIndex`; the flat-box dot
      // (a scatter overlay) carries the category index as value[0].
      const index = hit.seriesType === "scatter" ? (Array.isArray(hit.value) ? Number(hit.value[0]) : -1) : hit.dataIndex
      return typeof index === "number" && index >= 0 && index < categories.length ? named(categories[index]) : null
    }
    default:
      return named(hit.name)
  }
}
