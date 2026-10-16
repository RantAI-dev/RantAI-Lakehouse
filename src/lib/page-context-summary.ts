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

import type { FilterDef } from "@/services/clients/bi-store"
import { filterLabel, isActiveFilter, normalizeFilters } from "./dashboard-filter-state"
import { GRAIN_LABELS, isGrain, isTruncation, type Grain } from "./time-grain"

export type TileLike = {
  id: string
  title: string
  kind: string
  source?: string
  mart?: string
  sqlSource?: string
}

export type CellLike =
  | { columns: string[]; rows: Record<string, unknown>[] }
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
    if (t.kind === "text") data = "note"
    else if (!cell) data = "not loaded"
    else if ("error" in cell) data = `error: ${fmt(cell.error)}`
    else data = summarizeRows(cell.columns, cell.rows, maxRows)
    return `- "${t.title}" (${t.kind}${id}; ${from}): ${data}`
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
 * The dashboard's grain, in words the control already uses: "Day" is the
 * board's own grain, "Month (overrides saved Day)" is what the page shows
 * when the address picks one that is not the saved default, and "each
 * chart's own" is the empty choice (the chart picks its own truncation).
 * `shown` is what the page actually shows today; `saved` is the board's
 * default. The label reuses `GRAIN_LABELS` from the control so a reader
 * never meets a vocabulary the UI does not show.
 */
export function summarizeGrain(shown: Grain | "", saved: string | undefined): string {
  if (shown === "") return "Time grain: each chart's own"
  const shownLabel = GRAIN_LABELS[shown]
  const savedGrain = saved && isGrain(saved) && isTruncation(saved) ? saved : undefined
  if (savedGrain && savedGrain !== shown) {
    return `Time grain: ${shownLabel} (overrides saved ${GRAIN_LABELS[savedGrain]})`
  }
  return `Time grain: ${shownLabel}`
}

/**
 * The active filters as compact JSON, for the page context when there is
 * room (the chip text alone leaves the model guessing). The helpers in
 * `lib/dashboard-filter-state` normalize the list so equal filters compare
 * equal; an over-budget JSON is dropped (`null`) so the chip summary stays
 * the only filter context, never a half-truncated bracket.
 */
export function summarizeFilterJson(
  filters: FilterLike[],
  opts: { maxChars?: number } = {}
): string | null {
  const json = JSON.stringify(normalizeFilters(filters))
  const max = opts.maxChars ?? DEFAULT_CHARS
  return json.length <= max ? json : null
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
