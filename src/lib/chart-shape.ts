import type { FilterDef } from "@/services/clients/bi-store"

/**
 * The chart builder's own data fields (`BI-6`): a measure with its own
 * aggregate, the chart's own filters, a sort, the top or bottom N with an
 * "Other" group, and the groupings of a table. They mirror
 * `lakehouse_bi::shape` (flat, camelCase on the wire); the server checks
 * every rule at save, so this module names the shapes, builds the payload and
 * labels what comes back. Pure, so a test can pin each conversion without
 * rendering the dialog.
 */

/** A measure as stored: a column (the chart's aggregate applies) or a column with its own aggregate. */
export type MeasureEntry = string | { column: string; aggregate: string }
export type SortDir = "asc" | "desc"
export type SortSpec = { by: string; dir: SortDir }
export type Grouping = { column: string; grain?: string }

export type ShapeDefFields = {
  measures?: MeasureEntry[]
  filters?: FilterDef[]
  sort?: SortSpec
  other?: boolean
  groupings?: Grouping[]
}

/** Most columns a table may be grouped by. */
export const MAX_GROUPINGS = 5
/** The label of the computed group, and the column that marks it in a result. */
export const OTHER_LABEL = "Other"
export const OTHER_FLAG = "__other"
/** What the computed group is called when a real value in the data is also "Other". */
export const OTHER_REST_LABEL = "Other (rest)"

/** One measure row of the builder: a column, and an aggregate of its own ("" = the chart's). */
export type MeasureRow = { column: string; aggregate: string }

export const MEASURE_AGGREGATES = ["sum", "avg", "max", "min", "count"] as const

const AGGREGATE_WORDS: Record<string, string> = {
  sum: "sum", avg: "avg", max: "max", min: "min", count: "count",
}

/** The measures a saved definition lists, as builder rows. */
export function rowsFromEntries(entries: readonly MeasureEntry[] | undefined): MeasureRow[] {
  const rows = (entries ?? []).map((e) =>
    typeof e === "string" ? { column: e, aggregate: "" } : { column: e.column, aggregate: e.aggregate })
  return rows.length ? rows : [{ column: "", aggregate: "" }]
}

/** The `measures` to send: a name when the row has no aggregate of its own, an object when it has; empty rows are dropped. */
export function entriesFromRows(rows: readonly MeasureRow[], own: boolean): MeasureEntry[] {
  return rows
    .filter((r) => r.column)
    .map((r) => (own && r.aggregate ? { column: r.column, aggregate: r.aggregate } : r.column))
}

/**
 * The result name of a measure row: `avg_stay` for an own aggregate, the
 * column's name otherwise. A calculated field of aggregate or table level
 * is the whole measure and ignores the aggregate, so it keeps its name
 * (`lakehouse_bi::shape::resolve_measures`).
 */
export function resultName(row: MeasureRow, wholeMeasure: ReadonlySet<string>): string {
  if (!row.aggregate || wholeMeasure.has(row.column)) return row.column
  return `${row.aggregate}_${row.column}`
}

/** "avg of stay" for a measure with its own aggregate; the plain name otherwise. */
export function measureLabel(row: MeasureRow, wholeMeasure: ReadonlySet<string>): string {
  if (!row.aggregate || wholeMeasure.has(row.column)) return row.column
  return `${AGGREGATE_WORDS[row.aggregate] ?? row.aggregate} of ${row.column}`
}

/**
 * How many measures a chart takes, and whether they may carry an aggregate
 * of their own. A fixed role (X and Y of a scatter, bar and line of a combo)
 * is a fixed count; the grouped charts and tables take several.
 */
export function measureLimits(kind: string, rowsMode = false): { min: number; max: number; own: boolean } {
  if (rowsMode) return { min: 0, max: 0, own: false }
  switch (kind) {
    case "bar": case "hbar": case "line": case "area": case "table":
      return { min: 1, max: 6, own: true }
    case "stacked":
      return { min: 2, max: 6, own: true }
    case "combo": case "scatter":
      return { min: 2, max: 2, own: true }
    case "bubble":
      return { min: 3, max: 3, own: true }
    default:
      return { min: 1, max: 1, own: false }
  }
}

/** Kinds that can gather the rest of their groups into "Other" (`lakehouse_bi::shape::takes_other`). */
export function takesOther(kind: string, rowsMode = false): boolean {
  if (rowsMode) return false
  return ["bar", "hbar", "stacked", "pie", "rose", "funnel", "treemap", "table"].includes(kind)
}

/** Kinds whose statement is a plain `GROUP BY`, the only ones a sort applies to. */
export function takesSort(kind: string, rowsMode = false): boolean {
  if (rowsMode) return false
  return !["text", "kpi", "gauge", "boxplot", "pointmap", "geoheat", "pivot"].includes(kind)
}

// ── the sort choice ──────────────────────────────────────────────────────

/**
 * The builder's one sort control. The three first values are the order a
 * chart always had (sent as `order`); `by:<name>:<dir>` is a sort by a
 * grouping or one measure (sent as `sort`).
 */
export type SortChoice = string

export const DEFAULT_ORDERS: Record<string, string> = {
  "order:desc": "Highest first", "order:asc": "Lowest first", "order:none": "Natural (by dimension)",
}

export type SortOption = { value: SortChoice; label: string }

/** Every choice the sort control offers, for the chart's groupings and measures. */
export function sortOptions(groups: readonly string[], measures: readonly { name: string; label: string }[]): SortOption[] {
  const out: SortOption[] = Object.entries(DEFAULT_ORDERS).map(([value, label]) => ({ value, label }))
  for (const g of groups) {
    out.push({ value: `by:${g}:asc`, label: `${g}, A to Z` }, { value: `by:${g}:desc`, label: `${g}, Z to A` })
  }
  for (const m of measures) {
    out.push({ value: `by:${m.name}:desc`, label: `${m.label}, highest first` }, { value: `by:${m.name}:asc`, label: `${m.label}, lowest first` })
  }
  return out
}

/** The `order` or `sort` a choice stands for. */
export function parseSortChoice(choice: SortChoice): { order?: "desc" | "asc" | "none"; sort?: SortSpec } {
  if (choice.startsWith("order:")) {
    const order = choice.slice("order:".length)
    return order === "asc" || order === "none" ? { order } : { order: "desc" }
  }
  const match = /^by:(.*):(asc|desc)$/.exec(choice)
  return match ? { sort: { by: match[1], dir: match[2] as SortDir } } : { order: "desc" }
}

/** The choice a saved definition stands for. */
export function sortChoiceOf(def: { order?: string; sort?: SortSpec }): SortChoice {
  if (def.sort) return `by:${def.sort.by}:${def.sort.dir}`
  return `order:${def.order === "asc" || def.order === "none" ? def.order : "desc"}`
}

/** `order` in the words the limit and grain rules read: a sort by a grouping is "none", by a measure a value order. */
export function effectiveOrder(choice: SortChoice, groups: readonly string[]): "desc" | "asc" | "none" {
  const { order, sort } = parseSortChoice(choice)
  if (order) return order
  if (!sort) return "desc"
  return groups.includes(sort.by) ? "none" : sort.dir
}

// ── the draft ────────────────────────────────────────────────────────────

/** The builder's state for the `BI-6` fields other than the measures. */
export type ShapeDraft = {
  filters: FilterDef[]
  sortChoice: SortChoice
  other: boolean
  groupings: Grouping[]
}

export function emptyShapeDraft(): ShapeDraft {
  return { filters: [], sortChoice: "order:desc", other: false, groupings: [] }
}

/** A saved definition as a draft; anything absent keeps its default. */
export function shapeDraftFromDef(def: (ShapeDefFields & { order?: string }) | undefined): ShapeDraft {
  if (!def) return emptyShapeDraft()
  return {
    filters: (def.filters ?? []).map((f) => ({ ...f, values: [...(f.values ?? [])] })),
    sortChoice: sortChoiceOf(def),
    other: def.other === true,
    groupings: (def.groupings ?? []).map((g) => ({ ...g })),
  }
}

/** Which of the draft's fields this kind sends. */
export type ShapeContext = { kind: string; rowsMode: boolean; grained: boolean }

/**
 * The fields to merge into the payload. `order` is sent by the caller (it is
 * the legacy order); this adds `sort`, and drops what the kind or the other
 * choices do not allow, so a choice left over from another kind is not sent.
 */
export function shapePayload(d: ShapeDraft, ctx: ShapeContext): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  if (ctx.kind !== "text" && d.filters.length) out.filters = d.filters
  const { sort } = parseSortChoice(d.sortChoice)
  if (sort && takesSort(ctx.kind, ctx.rowsMode)) out.sort = sort
  if (d.other && takesOther(ctx.kind, ctx.rowsMode) && !ctx.grained && !(ctx.kind === "table" && d.groupings.length > 1)) out.other = true
  if (ctx.kind === "table" && !ctx.rowsMode && (d.groupings.length > 1 || d.groupings.some((g) => g.grain))) {
    out.groupings = d.groupings.map((g) => (g.grain ? { column: g.column, grain: g.grain } : { column: g.column }))
  }
  return out
}

/** Whether a table's groupings need the `groupings` field (several, or a grain), rather than the plain `dimension`. */
export function usesGroupings(groupings: readonly Grouping[]): boolean {
  return groupings.length > 1 || groupings.some((g) => !!g.grain)
}

/** What is still needed before the chart can be previewed, in the builder's own words. */
export function shapeProblems(d: ShapeDraft, ctx: { kind: string; rowsMode: boolean }): string[] {
  const out: string[] = []
  if (ctx.kind === "table" && !ctx.rowsMode && d.groupings.length === 0) out.push("Group by")
  return out
}

// ── what comes back ──────────────────────────────────────────────────────

/** Whether a result row is the computed "Other" group. */
export function isOtherRow(row: Record<string, unknown> | undefined): boolean {
  const flag = row?.[OTHER_FLAG]
  return flag === 1 || flag === "1" || flag === true
}

/**
 * The rows as drawn, and the labels of the computed groups among them. A real
 * value that is literally "Other" is a different group from the computed one
 * (the flag tells them apart), but both read "Other" on an axis; when a
 * result holds both, the computed group is drawn as "Other (rest)" so the two
 * are never merged into one bar and a click on the real value still finds it.
 */
export function labelOtherRows<R extends Record<string, unknown>>(rows: readonly R[], x: string): { rows: R[]; otherLabels: Set<string> } {
  const flagged = rows.filter((r) => isOtherRow(r))
  if (!flagged.length) return { rows: [...rows], otherLabels: new Set() }
  const realOther = rows.some((r) => !isOtherRow(r) && String(r[x] ?? "") === OTHER_LABEL)
  const label = realOther ? OTHER_REST_LABEL : OTHER_LABEL
  return {
    rows: rows.map((r) => (isOtherRow(r) ? { ...r, [x]: label } : r)),
    otherLabels: new Set([label]),
  }
}

/**
 * Result name → readable label ("avg of stay") for a tile, by position: the
 * tile's `y` names the result columns in the order of the definition's
 * measures, and a name that differs from its column is an own aggregate.
 */
export function tileMeasureLabels(measures: readonly MeasureEntry[] | undefined, y: string | readonly string[]): Record<string, string> {
  const names = Array.isArray(y) ? y : [y as string]
  const out: Record<string, string> = {}
  names.forEach((name, i) => {
    const entry = measures?.[i]
    if (entry && typeof entry !== "string" && name !== entry.column) {
      out[name] = `${AGGREGATE_WORDS[entry.aggregate] ?? entry.aggregate} of ${entry.column}`
    }
  })
  return out
}
