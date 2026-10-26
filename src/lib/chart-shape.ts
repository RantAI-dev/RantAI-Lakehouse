import type { FilterDef } from "@/services/clients/bi-store"
import { type BinDef, type BinDraft, type GridDef, binDraftFromDef, binPayload, binProblem, gridChoiceOf, gridPayload } from "./chart-bins"
import { type JoinDef, type JoinDraft, columnSource, displayColumn, displayColumnFull, joinDraftsFromDef, joinProblems, joinsPayload } from "./chart-joins"
import type { LayerDef } from "./chart-layers"

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
export type MeasureEntry = string | { column: string; aggregate: string } | { metric: string }

/**
 * A measure row that names a saved metric (BI-2) holds `metric:<name>` as its column, so the
 * pickers and rows need no second field; a column or calculated field never has a colon.
 */
export const METRIC_PREFIX = "metric:"
export const metricColumn = (name: string) => `${METRIC_PREFIX}${name}`
export const isMetricColumn = (column: string) => column.startsWith(METRIC_PREFIX)
export const metricNameOf = (column: string) => column.slice(METRIC_PREFIX.length)

/** What the builder knows about a column name that its spelling does not say. */
export type ColumnRefContext = {
  /** Label of a metric by its name (the server's catalogue). */
  readonly metricLabels?: Readonly<Record<string, string>>
  readonly calculated?: ReadonlySet<string>
  readonly groups?: ReadonlySet<string>
  readonly isParam?: (name: string) => boolean
}

/** A column reference as shown: the text, the small tag after it (where it comes from or what it is), and the full name for a hover title. */
export type ColumnRef = { text: string; tag: string | null; full: string }

/**
 * The one place that says how a column reference reads, for the pickers (list
 * and closed state), the section summaries and the labels of a chart (phase 2
 * QA R14). A joined column is its own name with its table as the tag
 * (`rooms` · capacity), a metric its label (tag "metric"), a calculated field
 * "fx", a value group "group", a parameter "param"; any other column is its name.
 */
export function columnRef(name: string, ctx: ColumnRefContext = {}): ColumnRef {
  if (isMetricColumn(name)) {
    const text = ctx.metricLabels?.[metricNameOf(name)] || metricNameOf(name)
    return { text, tag: "metric", full: text }
  }
  if (ctx.isParam?.(name)) return { text: name, tag: "param", full: name }
  if (ctx.calculated?.has(name)) return { text: name, tag: "fx", full: name }
  if (ctx.groups?.has(name)) return { text: name, tag: "group", full: name }
  return { text: displayColumn(name), tag: columnSource(name), full: displayColumnFull(name) }
}
export type SortDir = "asc" | "desc"
export type SortSpec = { by: string; dir: SortDir }
export type Grouping = { column: string; grain?: string; bin?: BinDef }

/** A condition of a filter step, on a column of the result before it (`BI-6` part 2). */
export type StepCondition = { column: string; op: string; value?: string; min?: string; max?: string }
export type StepMeasure = { column: string; aggregate: string }
/** A step: exactly one of `filter` (conditions, all must hold) and `summarise`. */
export type StepDef = { filter?: StepCondition[]; summarise?: { by: string[]; measures: StepMeasure[] } }
/** Most steps a chart takes. */
export const MAX_STEPS = 2

export type ShapeDefFields = {
  measures?: MeasureEntry[]
  /** The aggregate of a measure written as a plain name (phase 2 QA R4: the form shows it on each such measure). */
  aggregate?: string
  filters?: FilterDef[]
  sort?: SortSpec
  other?: boolean
  groupings?: Grouping[]
  steps?: StepDef[]
  /** Tables joined to the chart's mart (BI-7). */
  joins?: JoinDef[]
  /** How the number dimension (or a pivot's first number field) is binned, and the number breakdown (BI-10). */
  bin?: BinDef
  breakdownBin?: BinDef
  /** A point map grouped by grid cell (BI-10). */
  grid?: GridDef
  /** More questions drawn on the same chart (BI-13), and how its own series is drawn. */
  layers?: LayerDef[]
  draw?: "bar" | "line"
  axis?: "left" | "right"
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
    typeof e === "string"
      ? { column: e, aggregate: "" }
      : "metric" in e
        ? { column: metricColumn(e.metric), aggregate: "" }
        : { column: e.column, aggregate: e.aggregate })
  return rows.length ? rows : [{ column: "", aggregate: "" }]
}

/** The `measures` to send: a name when the row has no aggregate of its own, an object when it has; empty rows are dropped. */
export function entriesFromRows(rows: readonly MeasureRow[], own: boolean): MeasureEntry[] {
  return rows
    .filter((r) => r.column)
    .map((r): MeasureEntry =>
      isMetricColumn(r.column) ? { metric: metricNameOf(r.column) } : own && r.aggregate ? { column: r.column, aggregate: r.aggregate } : r.column)
}

/**
 * The result name of a measure row: `avg_stay` for an own aggregate, the
 * column's name otherwise. A calculated field of aggregate or table level
 * is the whole measure and ignores the aggregate, so it keeps its name
 * (`lakehouse_bi::shape::resolve_measures`).
 */
export function resultName(row: MeasureRow, wholeMeasure: ReadonlySet<string>): string {
  if (isMetricColumn(row.column)) return metricNameOf(row.column)
  if (!row.aggregate || wholeMeasure.has(row.column)) return row.column
  return `${row.aggregate}_${row.column}`
}

/** The aggregate a row computes: its own, or the chart's (`sum` when the chart names none). */
export function effectiveAggregate(row: MeasureRow, chartAggregate = "sum"): string {
  return row.aggregate || chartAggregate || "sum"
}

/**
 * What a row stores when an aggregate is picked on it: none when it is the chart's own, so the
 * row keeps the column's name as a result (`visitors`) and not `sum_visitors`, as before.
 */
export function storedAggregate(picked: string, chartAggregate: string): string {
  return picked === chartAggregate ? "" : picked
}

/**
 * What a measure row reads as: "avg of stay" when it was computed with an
 * aggregate that is not the column's plain sum, the column's name otherwise
 * (phase 2 QA R7). A row that takes the chart's aggregate says that aggregate
 * too, so a second measure left on a default of `avg` is no longer drawn as
 * the bare column. `chartAggregate` is the aggregate of rows that name none.
 */
export function measureLabel(row: MeasureRow, wholeMeasure: ReadonlySet<string>, chartAggregate = "sum"): string {
  if (isMetricColumn(row.column)) return metricNameOf(row.column)
  if (wholeMeasure.has(row.column)) return displayColumn(row.column)
  if (!row.aggregate) {
    return chartAggregate && chartAggregate !== "sum" ? `${AGGREGATE_WORDS[chartAggregate] ?? chartAggregate} of ${displayColumn(row.column)}` : displayColumn(row.column)
  }
  return `${AGGREGATE_WORDS[row.aggregate] ?? row.aggregate} of ${displayColumn(row.column)}`
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
    // BI-16 part B: a histogram counts the rows of its dimension and takes no measure.
    case "histogram":
    // BI-16 part C1 (plan D1): a Gantt chart has a start and an end instead of a measure.
    case "gantt":
      return { min: 0, max: 0, own: false }
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
  return !["text", "kpi", "gauge", "boxplot", "pointmap", "geoheat", "pivot", "progress", "histogram", "bullet", "image", "web", "gantt"].includes(kind)
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
  steps: StepDef[]
  /** Tables joined to the chart's mart (BI-7); only for a chart on a mart. */
  joins: JoinDraft[]
  /** BI-10: the bin of the dimension (or of a pivot's number field), of the breakdown, and the map grid ("none", "auto", "0.01" ...). */
  bin: BinDraft | null
  breakdownBin: BinDraft | null
  grid: string
}

export function emptyShapeDraft(): ShapeDraft {
  return { filters: [], sortChoice: "order:desc", other: false, groupings: [], steps: [], joins: [], bin: null, breakdownBin: null, grid: "none" }
}

/** A saved definition as a draft; anything absent keeps its default. */
export function shapeDraftFromDef(def: (ShapeDefFields & { order?: string }) | undefined): ShapeDraft {
  if (!def) return emptyShapeDraft()
  return {
    filters: (def.filters ?? []).map((f) => ({ ...f, values: [...(f.values ?? [])] })),
    sortChoice: sortChoiceOf(def),
    other: def.other === true,
    groupings: (def.groupings ?? []).map((g) => ({ ...g })),
    steps: structuredClone(def.steps ?? []),
    joins: joinDraftsFromDef(def.joins),
    bin: binDraftFromDef(def.bin),
    breakdownBin: binDraftFromDef(def.breakdownBin),
    grid: gridChoiceOf(def.grid),
  }
}

/** Which of the draft's fields this kind sends. */
export type ShapeContext = { kind: string; rowsMode: boolean; grained: boolean; breakdown?: boolean }

/** Kinds that take steps: the grouped charts and tables whose last result is a list of groups (`lakehouse_bi::shape::check_steps_kind`). */
export function takesSteps(kind: string, rowsMode = false): boolean {
  if (rowsMode) return false
  return ["bar", "hbar", "line", "area", "stacked", "combo", "pie", "rose", "funnel", "treemap", "radar", "waterfall", "table", "scatter", "bubble"].includes(kind)
}

/** Kinds whose dimension and breakdown can be binned (`lakehouse_bi::shape::takes_bin`). */
export function takesBin(kind: string, rowsMode = false): boolean {
  if (rowsMode) return false
  return ["bar", "hbar", "line", "area", "stacked", "combo", "pie", "rose", "funnel", "treemap", "radar", "waterfall", "table", "pivot"].includes(kind)
}

/** Whether the draft's steps are sent for this chart: not beside a time grain, a breakdown or "Other". */
export function stepsApply(d: ShapeDraft, ctx: ShapeContext): boolean {
  return d.steps.length > 0 && takesSteps(ctx.kind, ctx.rowsMode) && !ctx.grained && !ctx.breakdown && !d.other
}

/**
 * The fields to merge into the payload. `order` is sent by the caller (it is
 * the legacy order); this adds `sort`, and drops what the kind or the other
 * choices do not allow, so a choice left over from another kind is not sent.
 */
export function shapePayload(d: ShapeDraft, ctx: ShapeContext): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  if (ctx.kind !== "text" && d.filters.length) out.filters = d.filters
  if (ctx.kind !== "text") Object.assign(out, joinsPayload(d.joins))
  const { sort } = parseSortChoice(d.sortChoice)
  if (sort && takesSort(ctx.kind, ctx.rowsMode)) out.sort = sort
  if (stepsApply(d, ctx)) out.steps = d.steps.map(cleanStep)
  if (d.other && takesOther(ctx.kind, ctx.rowsMode) && !ctx.grained && !(ctx.kind === "table" && d.groupings.length > 1)) out.other = true
  if (ctx.kind === "table" && !ctx.rowsMode && usesGroupings(d.groupings)) {
    out.groupings = d.groupings.map((g) => {
      const bin = binPayload(binDraftFromDef(g.bin))
      return { column: g.column, ...(g.grain ? { grain: g.grain } : {}), ...(bin ? { bin } : {}) }
    })
  }
  // BI-10: a bin is for a number grouping and never beside a time grain; the grid is for the point maps.
  if (takesBin(ctx.kind, ctx.rowsMode) && !ctx.grained && ctx.kind !== "table") {
    const bin = binPayload(d.bin)
    if (bin) out.bin = bin
    const breakdownBin = ctx.breakdown ? binPayload(d.breakdownBin) : undefined
    if (breakdownBin) out.breakdownBin = breakdownBin
  }
  if (ctx.kind === "pointmap" || ctx.kind === "geoheat") {
    const grid = gridPayload(d.grid)
    if (grid) out.grid = grid
  }
  return out
}

/** Whether a table's groupings need the `groupings` field (several, or a grain), rather than the plain `dimension`. */
export function usesGroupings(groupings: readonly Grouping[]): boolean {
  return groupings.length > 1 || groupings.some((g) => !!g.grain || !!g.bin)
}

/** What is still needed before the chart can be previewed, in the builder's own words. */
export function shapeProblems(d: ShapeDraft, ctx: { kind: string; rowsMode: boolean; grained?: boolean; breakdown?: boolean }): string[] {
  const out: string[] = ctx.kind === "text" ? [] : joinProblems(d.joins)
  if (ctx.kind === "table" && !ctx.rowsMode && d.groupings.length === 0) out.push("Group by")
  // BI-10: a bin by width needs its width.
  if (takesBin(ctx.kind, ctx.rowsMode) && !ctx.grained) {
    const dimensionBin = binProblem(d.bin, "Dimension")
    if (dimensionBin) out.push(dimensionBin)
    const breakdownBin = ctx.breakdown ? binProblem(d.breakdownBin, "Breakdown") : null
    if (breakdownBin) out.push(breakdownBin)
  }
  if (ctx.kind === "table" && !ctx.rowsMode) {
    for (const g of d.groupings) {
      const problem = binProblem(binDraftFromDef(g.bin), g.column)
      if (problem) out.push(problem)
    }
  }
  if (stepsApply(d, { ...ctx, grained: !!ctx.grained })) {
    d.steps.forEach((step, i) => {
      const n = i + 1
      if (step.filter && !step.filter.length) out.push(`Step ${n} condition`)
      else if (step.filter && step.filter.some((c) => !conditionComplete(c))) out.push(`Step ${n} value`)
      if (step.summarise) {
        if (!step.summarise.by.length) out.push(`Step ${n} grouping`)
        if (!step.summarise.measures.length || step.summarise.measures.some((m) => !m.column)) out.push(`Step ${n} measure`)
      }
    })
  }
  return out
}

/** A condition that can be sent: a value (or a bound of a range) is filled in. */
export function conditionComplete(c: StepCondition): boolean {
  if (!c.column) return false
  if (c.op === "between") return !!(c.min?.trim() || c.max?.trim())
  return !!c.value?.trim()
}

/** A step as sent: empty fields dropped. */
export function cleanStep(step: StepDef): StepDef {
  if (step.summarise) return { summarise: { by: [...step.summarise.by], measures: step.summarise.measures.map((m) => ({ ...m })) } }
  return {
    filter: (step.filter ?? []).map((c) => (c.op === "between"
      ? { column: c.column, op: c.op, ...(c.min?.trim() ? { min: c.min.trim() } : {}), ...(c.max?.trim() ? { max: c.max.trim() } : {}) }
      : { column: c.column, op: c.op, value: (c.value ?? "").trim() })),
  }
}

// ── the columns after each step ──────────────────────────────────────────

/** The columns of a grouped result a step reads: groupings with their kinds, measures by result name, and their labels. */
export type ResultColumns = {
  groups: { name: string; kind: string }[]
  measures: string[]
  labels: Record<string, string>
  /** The source column each measure was computed from, so a later step can name its output plainly (phase 2 QA R8). */
  bases?: Record<string, string>
}

/** How a step's aggregate reads in front of a column: "total visitors", not "sum of avg of visitors". */
const STEP_WORDS: Record<string, string> = { sum: "total", avg: "average", max: "highest", min: "lowest", count: "count of" }

/**
 * The columns after `steps`, each applied to the result before it; a step that
 * names a column the result lacks keeps what it can. An output reads as the
 * aggregate in plain words and the original column ("total visitors"); two
 * outputs that would read the same keep their result names.
 */
export function columnsAfterSteps(base: ResultColumns, steps: readonly StepDef[]): ResultColumns {
  let cur = base
  for (const step of steps) {
    const sum = step.summarise
    if (!sum) continue
    const groups = sum.by.flatMap((b) => cur.groups.filter((g) => g.name === b))
    const measures: string[] = []
    const labels: Record<string, string> = {}
    const bases: Record<string, string> = {}
    for (const m of sum.measures) {
      if (!m.column || !m.aggregate) continue
      const name = `${m.aggregate}_${m.column}`
      measures.push(name)
      bases[name] = cur.bases?.[m.column] ?? m.column
      labels[name] = `${STEP_WORDS[m.aggregate] ?? m.aggregate} ${displayColumn(bases[name])}`
    }
    const seen = new Map<string, number>()
    for (const label of Object.values(labels)) seen.set(label, (seen.get(label) ?? 0) + 1)
    for (const name of measures) if ((seen.get(labels[name]) ?? 0) > 1) labels[name] = name
    cur = { groups, measures, labels, bases }
  }
  return cur
}

// ── what comes back ──────────────────────────────────────────────────────

/**
 * A result column whose name starts with two underscores is the server's own
 * (`__other`, the steps' and the ranking's working columns): it is not data
 * for a person, so no list of a tile's columns shows it or exports it.
 */
export function isInternalColumn(name: string): boolean {
  return name.startsWith("__")
}

/** The columns of a result a person reads: the internal ones left out. One helper for every place a tile's columns are listed. */
export function visibleColumns(columns: readonly string[]): string[] {
  return columns.filter((c) => !isInternalColumn(c))
}

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
 * BI-11 review fix (BLOCKER) R1: the definition a tile is read with. A stored
 * definition keeps a column switch as `{name}` text; the server says which
 * column each slot took in this response (`resolved`), and the tile's series
 * names, labels and clicks must follow that, not the unresolved text. Without
 * `resolved` the definition is returned as it is.
 */
export function withResolvedSlots<D extends { dimension?: string; breakdown?: string; measures?: MeasureEntry[] }>(
  def: D | undefined,
  resolved: { dimension?: string; breakdown?: string | null; measures?: unknown[] } | undefined,
): D | undefined {
  if (!def || !resolved) return def
  return {
    ...def,
    ...(resolved.dimension !== undefined ? { dimension: resolved.dimension } : {}),
    ...(resolved.breakdown !== undefined ? { breakdown: resolved.breakdown ?? undefined } : {}),
    ...(Array.isArray(resolved.measures) ? { measures: resolved.measures as MeasureEntry[] } : {}),
  }
}

/**
 * Result name → readable label ("avg of stay") for a tile, by position: the
 * tile's `y` names the result columns in the order of the definition's
 * measures, and a name that differs from its column is an own aggregate.
 * `metricLabels` is the tile payload's label of each metric by name.
 * `chartAggregate` is the chart's own aggregate, which a measure written as a
 * plain name takes: one that is not `sum` reads "avg of stay" too (phase 2 QA R7).
 */
export function tileMeasureLabels(
  measures: readonly MeasureEntry[] | undefined,
  y: string | readonly string[],
  steps?: readonly StepDef[],
  metricLabels?: Readonly<Record<string, string>>,
  chartAggregate?: string,
): Record<string, string> {
  const names = Array.isArray(y) ? y : [y as string]
  const out: Record<string, string> = {}
  // After steps the columns are the last step's, not the definition's measures by position.
  if (steps?.length) {
    const base: ResultColumns = {
      groups: [],
      measures: (measures ?? []).map((m) => (typeof m === "string" ? m : "metric" in m ? m.metric : `${m.aggregate}_${m.column}`)),
      labels: Object.fromEntries((measures ?? []).flatMap((m) => (typeof m === "string" || "metric" in m ? [] : [[`${m.aggregate}_${m.column}`, `${m.aggregate} of ${displayColumn(m.column)}`]]))),
      bases: Object.fromEntries((measures ?? []).flatMap((m) => (typeof m === "string" || "metric" in m ? [] : [[`${m.aggregate}_${m.column}`, m.column]]))),
    }
    const after = columnsAfterSteps({ ...base, groups: steps.flatMap((s) => (s.summarise?.by ?? []).map((name) => ({ name, kind: "text" }))) }, steps)
    for (const name of names) if (after.labels[name]) out[name] = after.labels[name]
    return out
  }
  names.forEach((name, i) => {
    const entry = measures?.[i]
    // BI-2 review fix (SHOULD-FIX) R3: a metric reads by its label ("Total
    // visitors"), which the server sends with the tile; with no label the name
    // stays, as before. Only a measure that is not an own aggregate can be one.
    const metricLabel = metricLabels?.[name]
    if (metricLabel && (!entry || typeof entry === "string" || "metric" in entry)) {
      out[name] = metricLabel
    } else if (entry && typeof entry !== "string" && !("metric" in entry) && name !== entry.column) {
      out[name] = `${AGGREGATE_WORDS[entry.aggregate] ?? entry.aggregate} of ${displayColumn(entry.column)}`
    } else if (entry && typeof entry === "string" && chartAggregate && chartAggregate !== "sum" && name === entry) {
      // Phase 2 QA R7: a plain name takes the chart's aggregate, which the label says.
      out[name] = `${AGGREGATE_WORDS[chartAggregate] ?? chartAggregate} of ${displayColumn(entry)}`
    } else if (typeof name === "string" && displayColumn(name) !== name) {
      // BI-7: a measure from a joined table reads table.column.
      out[name] = displayColumn(name)
    }
  })
  return out
}
