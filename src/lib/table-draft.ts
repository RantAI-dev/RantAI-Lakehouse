import {
  MAX_PIVOT_COLUMNS, MAX_PIVOT_ROWS, MAX_PIVOT_VALUES, MAX_TABLE_COLUMNS,
  type ColumnSetting, type ComparePeriod, type PivotTotals, type PivotValue, type TableDefFields,
} from "./table-types"

/**
 * The chart builder's state for raw tables, pivots and KPI comparisons
 * (`BI-16` part A), and the two conversions around it: a saved definition
 * into the draft, and the draft into the fields the server takes. Kept pure
 * so a test can pin both without rendering the dialog.
 */
export type TableDraft = {
  mode: "grouped" | "rows"
  /** Raw table: the columns, in order. */
  columns: string[]
  sortColumn: string
  sortDir: "asc" | "desc"
  pivotRows: string[]
  pivotColumns: string[]
  pivotValues: PivotValue[]
  totals: PivotTotals
  settings: Record<string, ColumnSetting>
  compareKind: "none" | "previous" | "goal"
  compareColumn: string
  comparePeriod: ComparePeriod
  /** Text, so a half-typed number is not lost. */
  goal: string
  goodDirection: "up" | "down"
}

export function emptyTableDraft(): TableDraft {
  return {
    mode: "grouped", columns: [], sortColumn: "", sortDir: "asc",
    pivotRows: [], pivotColumns: [], pivotValues: [{ column: "", aggregate: "sum" }], totals: "none",
    settings: {},
    compareKind: "none", compareColumn: "", comparePeriod: "month", goal: "", goodDirection: "up",
  }
}

/** A saved definition as a draft; anything absent keeps its default. */
export function draftFromDef(def: TableDefFields | undefined): TableDraft {
  const d = emptyTableDraft()
  if (!def) return d
  const compare = def.compare
  return {
    ...d,
    mode: def.tableMode === "rows" ? "rows" : "grouped",
    columns: def.tableMode === "rows" ? [...(def.columns ?? [])] : [],
    sortColumn: def.sortColumn ?? "",
    sortDir: def.sortDir === "desc" ? "desc" : "asc",
    pivotRows: [...(def.rows ?? [])],
    pivotColumns: def.rows ? [...(def.columns ?? [])] : [],
    pivotValues: def.values?.length ? def.values.map((v) => ({ ...v })) : d.pivotValues,
    totals: def.totals ?? "none",
    settings: Object.fromEntries(Object.entries(def.columnSettings ?? {}).map(([k, v]) => [k, { ...v }])),
    compareKind: compare?.kind ?? "none",
    compareColumn: compare?.kind === "previous" ? compare.dateColumn : "",
    comparePeriod: compare?.kind === "previous" ? (compare.period as ComparePeriod) : "month",
    goal: compare?.kind === "goal" ? String(compare.value) : "",
    goodDirection: def.goodDirection === "down" ? "down" : "up",
  }
}

/** A setting without anything to say is dropped, and a default is not stored. */
export function cleanSetting(s: ColumnSetting): ColumnSetting | null {
  const out: ColumnSetting = {}
  if (s.label?.trim()) out.label = s.label.trim()
  if (s.format && s.format !== "auto") out.format = s.format
  if (s.decimals !== undefined) out.decimals = s.decimals
  if (s.width !== undefined) out.width = s.width
  if (s.wrap) out.wrap = true
  if (s.hidden) out.hidden = true
  return Object.keys(out).length ? out : null
}

/** The settings worth sending: non-empty, and only for the columns the chart shows. */
export function cleanSettings(settings: Record<string, ColumnSetting>, shown: readonly string[]): Record<string, ColumnSetting> | undefined {
  const out: Record<string, ColumnSetting> = {}
  for (const column of shown) {
    const clean = settings[column] ? cleanSetting(settings[column]) : null
    if (clean) out[column] = clean
  }
  return Object.keys(out).length ? out : undefined
}

/** Move the item at `index` by `delta` places; out of range changes nothing. */
export function moveItem<T>(list: readonly T[], index: number, delta: number): T[] {
  const to = index + delta
  if (index < 0 || index >= list.length || to < 0 || to >= list.length) return [...list]
  const next = [...list]
  const [item] = next.splice(index, 1)
  next.splice(to, 0, item)
  return next
}

/** The fields to merge into the chart payload for `kind`; nothing for a kind that has none. */
export function draftPayload(kind: string, d: TableDraft, groupedColumns: readonly string[] = []): Record<string, unknown> {
  if (kind === "table") {
    if (d.mode === "rows") {
      return {
        tableMode: "rows", columns: d.columns,
        sortColumn: d.sortColumn || undefined, sortDir: d.sortColumn ? d.sortDir : undefined,
        columnSettings: cleanSettings(d.settings, d.columns),
      }
    }
    return { columnSettings: cleanSettings(d.settings, groupedColumns) }
  }
  if (kind === "pivot") {
    const values = d.pivotValues.filter((v) => v.column)
    return {
      rows: d.pivotRows,
      columns: d.pivotColumns.length ? d.pivotColumns : undefined,
      values, totals: d.totals,
      columnSettings: cleanSettings(d.settings, values.map((v) => v.column)),
    }
  }
  if (kind === "kpi") {
    const goal = Number(d.goal)
    return {
      compare: d.compareKind === "previous" && d.compareColumn
        ? { kind: "previous", dateColumn: d.compareColumn, period: d.comparePeriod }
        : d.compareKind === "goal" && d.goal.trim() !== "" && Number.isFinite(goal)
          ? { kind: "goal", value: goal }
          : undefined,
      goodDirection: d.compareKind !== "none" && d.goodDirection === "down" ? "down" : undefined,
    }
  }
  return {}
}

/** What is still needed before the chart can be previewed, in the builder's own words. */
export function draftProblems(kind: string, d: TableDraft): string[] {
  if (kind === "table" && d.mode === "rows") {
    return d.columns.length ? [] : ["Columns"]
  }
  if (kind === "pivot") {
    return [
      ...(d.pivotRows.length ? [] : ["Rows"]),
      ...(d.pivotValues.some((v) => v.column) ? [] : ["Values"]),
    ]
  }
  if (kind === "kpi") {
    if (d.compareKind === "previous" && !d.compareColumn) return ["Date column"]
    if (d.compareKind === "goal" && (d.goal.trim() === "" || !Number.isFinite(Number(d.goal)))) return ["Goal"]
  }
  return []
}

/** The limits the picker enforces, in one place for the components and the tests. */
export const PICK_LIMITS = {
  columns: MAX_TABLE_COLUMNS, pivotRows: MAX_PIVOT_ROWS, pivotColumns: MAX_PIVOT_COLUMNS, pivotValues: MAX_PIVOT_VALUES,
} as const
