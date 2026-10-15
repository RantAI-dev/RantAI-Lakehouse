/**
 * Raw tables, pivot tables and KPI comparisons (`BI-16` part A): the fields
 * they add to a chart definition. They mirror `lakehouse_bi::tables::TableFields`
 * (flat, camelCase on the wire); the server checks every limit at save, this
 * module only names the shapes.
 */

export const COLUMN_FORMATS = ["auto", "number", "percent", "currency", "date", "link", "image"] as const
export type ColumnFormat = (typeof COLUMN_FORMATS)[number]

/** One column's display settings; the server stores them, the console formats. */
export type ColumnSetting = {
  label?: string
  format?: ColumnFormat | string
  /** Digits after the point, 0 to 6. */
  decimals?: number
  /** Pixels, 60 to 800. */
  width?: number
  wrap?: boolean
  hidden?: boolean
}

export const AGGREGATES = ["sum", "avg", "max", "min", "count"] as const
export type PivotAggregate = (typeof AGGREGATES)[number]
export type PivotValue = { column: string; aggregate: PivotAggregate | string }
export type PivotTotals = "none" | "grand" | "all"

export const COMPARE_PERIODS = ["day", "week", "month", "quarter", "year"] as const
export type ComparePeriod = (typeof COMPARE_PERIODS)[number]
export type KpiCompare =
  | { kind: "previous"; dateColumn: string; period: ComparePeriod | string }
  | { kind: "goal"; value: number }

export type TableDefFields = {
  /** `table`: absent or `grouped` is the grouped summary. */
  tableMode?: "grouped" | "rows"
  /** `table` in rows mode: the columns shown, in order. `pivot`: the column fields (0 to 2). */
  columns?: string[]
  /** `pivot`: the row fields (1 to 3). */
  rows?: string[]
  /** `pivot`: the values (1 to 5). */
  values?: PivotValue[]
  totals?: PivotTotals
  sortColumn?: string
  sortDir?: "asc" | "desc"
  columnSettings?: Record<string, ColumnSetting>
  compare?: KpiCompare
  goodDirection?: "up" | "down"
}

/** Most columns a raw table lists, and the most a pivot has of each part. */
export const MAX_TABLE_COLUMNS = 30
export const MAX_PIVOT_ROWS = 3
export const MAX_PIVOT_COLUMNS = 2
export const MAX_PIVOT_VALUES = 5
