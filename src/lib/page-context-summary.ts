/**
 * Text summaries of what is on screen, for the Copilot's page context
 * ("page-aware" Copilot, plan §6). The model gets the numbers the user is
 * looking at — each tile's first rows, the filters in force, the SQL and
 * the first result rows in Query Studio — instead of tile titles alone.
 *
 * Text, not screenshots: `lakehouse-llm` sends text-only messages, and text
 * is cheaper and exact. Nothing here fetches: it summarizes data the page
 * already loaded. Every summary is bounded by `maxChars`; the API then
 * applies its own hard cap (`PAGE_CONTEXT_MAX_CHARS` in routes/ai/mod.rs).
 */

import { isStaticKind } from "./dashboard-tabs"
import type { FilterDef } from "@/services/clients/bi-store"
import { type BinDef, type BinMeta, binLabel, binsOfTile } from "./chart-bins"
import { filterLabel, isActiveFilter } from "./dashboard-filter-state"

export type TileLike = {
  id: string
  title: string
  kind: string
  source?: string
  mart?: string
  sqlSource?: string
  /** BI-6: the chart's own filters, which the numbers on the tile already carry. BI-10: its bins. */
  def?: { filters?: FilterDef[]; dimension?: string; bin?: BinDef; breakdown?: string; breakdownBin?: BinDef; groupings?: { column: string; bin?: BinDef }[] }
}

/** One more question on a chart (BI-13), as the tile carries its answer. */
export type LayerLike = { label: string; draw: string; axis: string; source: string; columns: string[]; rows: Record<string, unknown>[]; error?: string }

export type CellLike =
  | { columns: string[]; rows: Record<string, unknown>[]; bins?: Record<string, BinMeta>; layers?: LayerLike[] }
  | { error: string }
  | undefined
  | null

export type FilterLike = FilterDef

const DEFAULT_ROWS = 5
const DEFAULT_CHARS = 5000
const VALUE_CHARS = 40

/** One value, compact: numbers as-is, text clipped, empties as "∅". */
function fmt(value: unknown): string {
  if (value === null || value === undefined || value === "") return "∅"
  const s = typeof value === "number" ? String(value) : String(value).replace(/\s+/g, " ")
  return s.length > VALUE_CHARS ? `${s.slice(0, VALUE_CHARS - 1)}…` : s
}

function rowText(columns: string[], row: Record<string, unknown>): string {
  // Two columns (category → value) read best as "a=1"; wider rows as "{c=v, …}".
  if (columns.length === 2) return `${fmt(row[columns[0]])}=${fmt(row[columns[1]])}`
  return `{${columns.map((c) => `${c}=${fmt(row[c])}`).join(", ")}}`
}

/** "rows: a=1, b=2 (+8 more)" for a result, bounded to `maxRows`. */
export function summarizeRows(
  columns: string[],
  rows: Record<string, unknown>[],
  maxRows = DEFAULT_ROWS
): string {
  if (rows.length === 0) return "no rows"
  const shown = rows.slice(0, maxRows).map((r) => rowText(columns, r)).join(", ")
  const more = rows.length > maxRows ? ` (+${rows.length - maxRows} more)` : ""
  return `rows: ${shown}${more}`
}

function clip(text: string, maxChars: number): string {
  return text.length > maxChars ? `${text.slice(0, maxChars - 13)}… (truncated)` : text
}

/** One line per tile: what it is, where its data comes from, and its first rows. */
export function summarizeTiles(
  tiles: TileLike[],
  results: Record<string, CellLike>,
  opts: { maxRows?: number; maxChars?: number } = {}
): string {
  const maxRows = opts.maxRows ?? DEFAULT_ROWS
  const lines = tiles.map((t) => {
    const from = t.sqlSource ? `SQL source ${t.sqlSource}` : t.mart ? t.mart : "no data source"
    const id = t.source && t.source !== "builtin" ? `, id ${t.id}` : ", built-in"
    const cell = results[t.id]
    let data: string
    if (isStaticKind(t.kind)) data = "note"
    else if (!cell) data = "not loaded"
    else if ("error" in cell) data = `error: ${fmt(cell.error)}`
    else data = summarizeRows(cell.columns, cell.rows, maxRows)
    // The numbers carry the chart's own filters; the assistant must know they do.
    const own = (t.def?.filters ?? []).filter(isActiveFilter).map((f) => filterLabel(f))
    const filtered = own.length ? `; filtered: ${own.join(", ")}` : ""
    // BI-10: a binned column holds each bin's lower edge; say what the numbers stand for.
    const bins = cell && "rows" in cell ? Object.entries(binsOfTile(cell.bins, t.def)).map(([column, b]) => `${column} in bins of ${binLabel(0, b.width).split(" – ")[1]} (a value is the bin's lower edge)`) : []
    const binned = bins.length ? `; binned: ${bins.join(", ")}` : ""
    // BI-13: the chart's other questions, each from its own source, drawn on the same axis.
    const layers = cell && "rows" in cell ? (cell.layers ?? []).map((l) => ` | also "${l.label}" (${l.source}; ${l.draw}, ${l.axis} axis): ${l.error ? `error: ${fmt(l.error)}` : summarizeRows(l.columns, l.rows, maxRows)}`) : []
    return `- "${t.title}" (${t.kind}${id}; ${from}${filtered}${binned}): ${data}${layers.join("")}`
  })
  return clip(lines.join("\n"), opts.maxChars ?? DEFAULT_CHARS)
}

/**
 * "region is north, south; date in the last 30 days" — or "none". Each
 * active filter reads as its chip does, so the assistant sees what the user
 * sees.
 */
export function summarizeFilters(filters: FilterLike[]): string {
  const parts = filters.filter(isActiveFilter).map((f) => filterLabel(f))
  return parts.length ? parts.join("; ") : "none"
}

/**
 * "target = 1000 (number); pick = rooms (list)" — or "none". The parameters the
 * dashboard shows with the value in force (BI-11), so the assistant reuses them
 * (`list_parameters` has the rest) instead of defining a second one.
 */
export function summarizeParams(controls: { param: { name: string; type: string }; value: string | number }[]): string {
  const parts = controls.map((c) => `${c.param.name} = ${fmt(c.value)} (${c.param.type})`)
  return parts.length ? parts.join("; ") : "none"
}

/**
 * The dashboard's tabs for the assistant (BI-18 part C, AI-9): each with its
 * id, whether it is the open one, and the chart ids on it, so "put the map
 * charts in a separate tab" can be done with `move_charts_to_tab`.
 */
export function summarizeTabs(
  tabs: readonly { id: string; title: string }[],
  tabOfChart: (id: string) => string | undefined,
  chartIds: readonly string[],
  selected?: string,
): string {
  if (tabs.length === 0) return "none (the dashboard has no tabs)"
  return tabs
    .map((t) => {
      const ids = chartIds.filter((id) => tabOfChart(id) === t.id)
      return `"${t.title}" (id ${t.id}${t.id === selected ? ", open" : ""}; ${ids.length ? `charts ${ids.join(", ")}` : "no charts"})`
    })
    .join("; ")
}

/** The SQL in the editor plus the first rows of its last result. */
export function summarizeQuery(
  sql: string,
  result: { columns: string[]; rows: Record<string, unknown>[]; rowCount?: number } | null,
  opts: { maxRows?: number; maxChars?: number } = {}
): string {
  const maxChars = opts.maxChars ?? DEFAULT_CHARS
  const sqlText = clip(sql.trim(), Math.floor(maxChars / 2))
  const res = result
    ? `Last result: ${result.rowCount ?? result.rows.length} row(s), columns ${result.columns.join(", ")}; ${summarizeRows(result.columns, result.rows, opts.maxRows ?? DEFAULT_ROWS)}`
    : "Not run yet."
  return clip(`SQL in the editor:\n${sqlText}\n${res}`, maxChars)
}
