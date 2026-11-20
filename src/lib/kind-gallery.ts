import { measureLimits } from "./chart-shape"
import type { ChartKind } from "./dashboard-specs"

/**
 * `BI-58` T3: the chart-type gallery of the builder's canvas (decision L2,
 * owner 2026-11-14). Each type is drawn as a small picture of the chosen
 * data, from the rows the live preview already fetched: no extra query, at
 * most `GALLERY_ROW_CAP` rows. Pure, so which type draws and which one is
 * greyed out is pinned by a test.
 *
 * Two questions, kept apart:
 * - `kindNeed`: can the chosen source make this type at all? It reads only
 *   what the builder's own pickers require (the measure count of
 *   `measureLimits`, the second column of a breakdown kind, the date column
 *   of a calendar or a Gantt chart, the number of a histogram, the region of
 *   a map). A type that cannot is greyed out with that one line.
 * - `drawsFrom`: can the preview's rows be drawn as this type without a new
 *   query? Only types that read the same shape of result: one row per X with
 *   its values, or (with a split) one row per X and series. The others show
 *   their icon until they are picked and the preview asks for them.
 */

/** The eight types in the canvas toolbar, the most used ones (layout A); the rest are behind "More". */
export const MAIN_KINDS: readonly ChartKind[] = ["bar", "hbar", "line", "area", "pie", "treemap", "kpi", "table"]

/** The most rows of the preview a gallery picture draws (feature page, Limits). */
export const GALLERY_ROW_CAP = 200

/** What the chosen source has, counted as the builder's pickers count it. */
export type SourceFacts = {
  /** Date and timestamp columns. */
  dates: number
  /** Columns a chart can group by without bins: dates and text. */
  groups: number
  /** Number columns and saved metrics: what a value can be. */
  numbers: number
}

const NO_DATA: readonly string[] = ["text", "image", "web"]
const NO_VALUE_TYPES: readonly string[] = ["table", "pivot", "gantt", "histogram", ...NO_DATA]

const plural = (n: number, one: string, many: string) => (n === 1 ? `a ${one}` : `${n} ${many}`)

/** Why the chosen source cannot make `kind`, in one line; null when it can, or when no source is chosen yet. */
export function kindNeed(kind: ChartKind, facts: SourceFacts | null): string | null {
  if (!facts || NO_DATA.includes(kind)) return null
  if ((kind === "calendar" || kind === "gantt") && facts.dates < 1) return "Needs a date column"
  if (kind === "histogram" && facts.numbers < 1) return "Needs a number column"
  // A breakdown kind groups by two columns (`needsBreakdown` in the builder).
  if ((kind === "heatmap" || kind === "sankey" || kind === "sunburst") && facts.groups < 2) return "Needs two text or date columns"
  if (kind === "geomap" && facts.groups < 1) return "Needs a column of region names"
  if (NO_VALUE_TYPES.includes(kind)) return null
  const min = Math.max(measureLimits(kind).min, 1)
  return facts.numbers < min ? `Needs ${plural(min, "number column", "number columns")}` : null
}

/** One row per X with its values. */
const CATEGORY_KINDS: readonly ChartKind[] = ["bar", "hbar", "line", "area", "pie", "rose", "funnel", "treemap", "waterfall", "radar", "table"]
/** One row per X and series (a split). */
const SERIES_KINDS: readonly ChartKind[] = ["bar", "hbar", "line", "area", "heatmap", "sankey", "sunburst"]

export type PreviewShape = { shape: "category" | "series"; values: number } | null

/**
 * The shape of the preview's result, from the spec it was drawn with: null
 * when it is no shape another type can be drawn from (a single number, a
 * map, a raw table, a combined chart, a failure).
 */
export function previewShape(spec: { kind: string; series?: string; y?: string | string[] }, hasLayers: boolean): PreviewShape {
  if (hasLayers) return null
  const values = Array.isArray(spec.y) ? spec.y.length : spec.y ? 1 : 0
  if (values < 1) return null
  if (spec.series) return SERIES_KINDS.includes(spec.kind as ChartKind) ? { shape: "series", values } : null
  return CATEGORY_KINDS.includes(spec.kind as ChartKind) || spec.kind === "stacked" || spec.kind === "combo" ? { shape: "category", values } : null
}

/** Whether the preview's rows can be drawn as `kind` as they are. */
export function drawsFrom(kind: ChartKind, shape: PreviewShape): boolean {
  if (!shape) return false
  if (shape.shape === "series") return SERIES_KINDS.includes(kind)
  if (kind === "stacked") return shape.values >= 2
  if (kind === "combo") return shape.values === 2
  return CATEGORY_KINDS.includes(kind)
}
