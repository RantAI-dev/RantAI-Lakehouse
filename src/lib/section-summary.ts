import { GRAIN_LABELS, type Grain } from "./time-grain"
import { displayColumn } from "./chart-joins"

/**
 * The one-line summaries of the chart builder's folded sections (phase 2 QA
 * R10) and the section a still-missing field belongs to. Pure, so every kind's
 * wording is pinned by a test and the builder carries no string building. The
 * caller passes labels as the rest of the builder shows them ("avg of
 * visitors", `table.column` for a joined column): this module only joins them.
 */
export type SummaryInput = {
  kind: string
  /** A raw-rows table. */
  rowsMode: boolean
  /** The source as the picker names it ("mart_visits", "SQL · Daily visits"); empty when none is chosen. */
  source: string
  joins: number
  /** Column names as stored; shown through `displayColumn`. */
  dimension: string
  grain: string
  breakdown: string
  /** A grouped table's groupings. */
  groupings: readonly string[]
  /** A raw table's columns. */
  rowColumns: readonly string[]
  pivot: { rows: readonly string[]; columns: readonly string[]; values: readonly string[] }
  /** Measure labels as the builder shows them, the filled rows only. */
  measures: readonly string[]
  /** The map's label (map kinds). */
  mapLabel: string
  lat: string
  lon: string
  /** Section 3. `sort` is the chosen option's label, empty when the kind has no sort control. */
  sort: string
  /** The limit as shown; "param" when it follows a parameter; null when the kind has none. */
  limit: number | "param" | null
  filters: number
  other: boolean
}

export type SectionSummaries = { data: string; show: string; sortAndFilter: string }

export const NOT_SET = "Not set yet"

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`
const list = (names: readonly string[]) => names.map(displayColumn).join(", ")

export function sectionSummaries(i: SummaryInput): SectionSummaries {
  const data = i.source ? [i.source, ...(i.joins ? [plural(i.joins, "join")] : [])].join(" · ") : NOT_SET
  return { data, show: showSummary(i), sortAndFilter: sortSummary(i) }
}

function showSummary(i: SummaryInput): string {
  const measures = i.measures.length ? [i.measures.map(displayColumn).join(", ")] : []
  const parts: string[] = []
  if (i.kind === "table" && i.rowsMode) {
    if (i.rowColumns.length) parts.push(`rows: ${plural(i.rowColumns.length, "column")}`)
  } else if (i.kind === "table") {
    if (i.groupings.length) parts.push(list(i.groupings))
    parts.push(...measures)
  } else if (i.kind === "pivot") {
    if (i.pivot.rows.length) parts.push(`rows ${list(i.pivot.rows)}`)
    if (i.pivot.columns.length) parts.push(`columns ${list(i.pivot.columns)}`)
    if (i.pivot.values.length) parts.push(`values ${list(i.pivot.values)}`)
  } else if (i.kind === "kpi" || i.kind === "gauge" || i.kind === "progress") {
    parts.push(...measures)
  } else if (i.kind === "geomap") {
    if (i.mapLabel) parts.push(i.mapLabel)
    if (i.dimension) parts.push(displayColumn(i.dimension))
    parts.push(...measures)
  } else if (i.kind === "pointmap" || i.kind === "geoheat") {
    if (i.mapLabel) parts.push(i.mapLabel)
    if (i.lat && i.lon) parts.push(`${displayColumn(i.lat)}, ${displayColumn(i.lon)}`)
    parts.push(...measures)
  } else if (i.kind !== "text" && i.kind !== "image" && i.kind !== "web") {
    if (i.dimension) {
      const by = i.grain ? ` by ${GRAIN_LABELS[i.grain as Grain]?.toLowerCase() ?? i.grain}` : ""
      parts.push(`${displayColumn(i.dimension)}${by}`)
    }
    if (i.breakdown) parts.push(`split by ${displayColumn(i.breakdown)}`)
    parts.push(...measures)
  }
  return parts.length ? parts.join(" · ") : NOT_SET
}

function sortSummary(i: SummaryInput): string {
  const parts: string[] = []
  if (i.sort) parts.push(i.sort.charAt(0).toLowerCase() + i.sort.slice(1))
  if (i.limit === "param") parts.push("top N from a parameter")
  else if (i.limit !== null) parts.push(`top ${i.limit}`)
  if (i.other) parts.push("rest as Other")
  parts.push(i.filters ? plural(i.filters, "filter") : "no filters")
  return parts.join(" · ")
}

/**
 * The builder's sections, by number: 1 Data, 2 What to show, 3 Sort and filter,
 * 4 More options. (Section 5, Format, holds nothing a save needs, so no missing field points to it.)
 */
export type SectionNumber = 1 | 2 | 3 | 4

/**
 * Which section holds a field the builder lists as missing (`missing` in the
 * builder, the words of `shapeProblems`, `draftProblems`, `layerProblems` and
 * `joinProblems`). Nothing in sort and filter is required.
 */
export function missingSection(label: string): SectionNumber {
  if (label === "Data source" || /^Join \d/.test(label)) return 1
  // A step, an extra question and a KPI's comparison all live under More options.
  if (/^(Step|Question) \d/.test(label) || label === "Date column" || label === "Goal") return 4
  return 2
}
