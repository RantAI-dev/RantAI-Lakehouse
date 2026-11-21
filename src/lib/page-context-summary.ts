/**
 * Text summaries of what is on screen, for the Copilot's page context
 * ("page-aware" Copilot, plan §6). The model gets the numbers the user is
 * looking at — each tile's first rows, the filters in force, the SQL and
 * the first result rows in Query Studio — instead of tile titles alone.
 *
 * Text, not screenshots: `lakehouse-llm` sends text-only messages, and text
 * is cheaper and exact. Nothing here fetches: it summarizes data the page
 * already loaded. Every summary is bounded by `maxChars`, and
 * `buildDashboardPageContext` fits the dashboard's whole context into the
 * API's hard cap (`PAGE_CONTEXT_MAX_CHARS` in routes/ai/mod.rs).
 */

import { isStaticKind } from "./dashboard-tabs"
import type { FilterDef } from "@/services/clients/bi-store"
import { type BinDef, type BinMeta, binLabel, binsOfTile } from "./chart-bins"
import { filterLabel, isActiveFilter, normalizeFilters } from "./dashboard-filter-state"
import { GRAIN_LABELS, isGrain, isTruncation, type Grain } from "./time-grain"
import { grainNotAppliedSentence } from "./grain-switch"

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
  | { columns: string[]; rows: Record<string, unknown>[]; bins?: Record<string, BinMeta>; layers?: LayerLike[]; grainNotApplied?: string }
  | { error: string }
  | undefined
  | null

export type FilterLike = FilterDef

const DEFAULT_ROWS = 5
const DEFAULT_CHARS = 5000
const VALUE_CHARS = 40
/** The tab list's own budget, and where the chart-to-tab mapping is read. */
const TAB_MAX_CHARS = 600
/** The parameter line's own budget: it plays the part for the parameter line that `TAB_MAX_CHARS` plays for the tabs line. */
const PARAM_MAX_CHARS = 600
const TAB_HINT = "; list_charts gives each chart's tab"
/**
 * How many characters of page context the API keeps, mirrored from
 * `rust/crates/lakehouse-api/src/routes/ai/mod.rs:238`: the server takes
 * the first this many characters and drops the rest, so a context longer
 * than this loses its ending.
 */
export const PAGE_CONTEXT_MAX_CHARS = 6000
/** What the model reads when the tile list had to be cut. */
const TILE_NOTE = "… (more tiles; list_charts lists every chart)"
/** What the model reads when the filter list had to be cut. */
const FILTER_NOTE = "… (more filters)"

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
    // BI-50 (QA LH-BI-9-05): why the dashboard's date grouping left this chart
    // as it was, so "why did this chart not change to monthly?" is answerable
    // from what the page sent.
    const grouping = cell && "rows" in cell ? grainNotAppliedSentence(cell.grainNotApplied) : null
    const grouped = grouping ? `; date grouping: ${grouping}` : ""
    // BI-13: the chart's other questions, each from its own source, drawn on the same axis.
    const layers = cell && "rows" in cell ? (cell.layers ?? []).map((l) => ` | also "${l.label}" (${l.source}; ${l.draw}, ${l.axis} axis): ${l.error ? `error: ${fmt(l.error)}` : summarizeRows(l.columns, l.rows, maxRows)}`) : []
    return `- "${t.title}" (${t.kind}${id}; ${from}${filtered}${binned}${grouped}): ${data}${layers.join("")}`
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

/**
 * "target = 1000 (number); pick = rooms (list)" — or "none". The parameters the
 * dashboard shows with the value in force (BI-11), so the assistant reuses them
 * (`list_parameters` has the rest) instead of defining a second one.
 *
 * The line has its own budget (`PARAM_MAX_CHARS`), like the tabs: a board with
 * many shown parameters must not push the dashboard's whole context past the
 * API's cap, which would cut off the board id and the closing instruction.
 * When the entries do not fit whole, the first ones that fit stay and a note
 * counts the rest, toward the budget; a single entry too long for the budget
 * is cut with `clip`.
 */
export function summarizeParams(
  controls: { param: { name: string; type: string }; value: string | number }[],
  opts: { maxChars?: number } = {}
): string {
  if (controls.length === 0) return "none"
  const maxChars = opts.maxChars ?? PARAM_MAX_CHARS
  const entries = controls.map((c) => `${c.param.name} = ${fmt(c.value)} (${c.param.type})`)
  const whole = entries.join("; ")
  if (whole.length <= maxChars) return whole
  // The note is part of the budget: it counts the entries the line leaves out.
  const note = (leftOut: number): string => (leftOut > 0 ? `; … (+${leftOut} more)` : "")
  // Not even the first entry fits beside its note: cut it, reserving the note's room.
  const firstNote = note(entries.length - 1)
  if (entries[0].length + firstNote.length > maxChars) {
    return clip(entries[0], maxChars - firstNote.length) + firstNote
  }
  // Keep the most entries whose joined line plus the note stays inside the budget.
  let kept = entries.length - 1
  while (kept > 1 && entries.slice(0, kept).join("; ").length + note(entries.length - kept).length > maxChars) {
    kept--
  }
  return entries.slice(0, kept).join("; ") + note(entries.length - kept)
}

/**
 * The dashboard's tabs for the assistant (BI-18 part C, AI-9): each with its
 * id, whether it is the open one, and the chart ids on it, so "put the map
 * charts in a separate tab" can be done with `move_charts_to_tab`.
 *
 * A board with many charts lists too long to send, so over the budget each
 * tab carries the number of charts on it instead of their ids, and the
 * summary says where the ids can be read.
 */
export function summarizeTabs(
  tabs: readonly { id: string; title: string }[],
  tabOfChart: (id: string) => string | undefined,
  chartIds: readonly string[],
  selected?: string,
  opts: { maxChars?: number } = {}
): string {
  if (tabs.length === 0) return "none (the dashboard has no tabs)"
  const maxChars = opts.maxChars ?? TAB_MAX_CHARS
  const tabLine = (t: { id: string; title: string }, charts: string) =>
    `"${t.title}" (id ${t.id}${t.id === selected ? ", open" : ""}; ${charts})`
  const perTab = tabs.map((t) => ({
    tab: t,
    charts: chartIds.filter((id) => tabOfChart(id) === t.id),
  }))
  const full = perTab
    .map(({ tab, charts }) => tabLine(tab, charts.length ? `charts ${charts.join(", ")}` : "no charts"))
    .join("; ")
  if (full.length <= maxChars) return full
  // Over the budget: count the charts per tab instead of naming them.
  const counted = perTab.map(({ tab, charts }) =>
    tabLine(tab, charts.length ? `${charts.length} chart${charts.length === 1 ? "" : "s"}` : "no charts")
  )
  // Tabs that do not fit are left out, so the list stays inside the budget.
  const kept: string[] = []
  for (const line of counted) {
    if ([...kept, line].join("; ").length > maxChars) break
    kept.push(line)
  }
  const list = kept.length ? kept.join("; ") : clip(counted[0], maxChars)
  return `${list}${TAB_HINT}`
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

/** The dashboard's context for the assistant, as the parts the page summarized. */
export type DashboardContextParts = {
  /** The dashboard's name, as the page shows it. */
  dashName: string
  /** The board id, the assistant's handle on this dashboard. */
  board: string
  /** The grain in the control's own words (`summarizeGrain`). */
  grainText: string
  /** The active filters as their chips read (`summarizeFilters`). */
  filterChips: string
  /** The active filters as compact JSON, or `null` when the page dropped it. */
  filterJson: string | null
  /** The parameters the dashboard shows (`summarizeParams`). */
  params: string
  /** The dashboard's tabs (`summarizeTabs`). */
  tabs: string
  /** The tiles with the data they show (`summarizeTiles`). */
  tiles: string
}

/**
 * Cut `text` to `maxChars` and end it with `note`, so the reader knows
 * something was left out and where to find it. The note counts toward
 * `maxChars`, so the result is never longer than the budget.
 */
function cutWithNote(text: string, maxChars: number, note: string): string {
  if (text.length <= maxChars) return text
  return `${text.slice(0, Math.max(0, maxChars - note.length))}${note}`
}

/**
 * The dashboard's context for the assistant: board and grain, filters,
 * parameters, tabs, tiles, and the instruction at the end.
 *
 * The server keeps the first `PAGE_CONTEXT_MAX_CHARS` characters and drops
 * the rest, so a long context would lose the instruction that carries the
 * board id. This fits the whole context into that budget by giving up the
 * raw filter JSON first, then the end of the tile list, then the filter
 * chip text. The first sentence, the grain, the parameters, the tabs and
 * the instruction are never cut.
 */
export function buildDashboardPageContext(parts: DashboardContextParts): string {
  const { dashName, board, grainText, filterChips, filterJson, params, tabs, tiles } = parts
  const render = (chips: string, json: string | null, tileContent: string): string =>
    `The user is viewing the dashboard "${dashName}" (board id: ${board}). ` +
    `${grainText}. ` +
    `Active filters: ${chips}${json ? ` (raw: ${json})` : ""}. ` +
    `Parameters shown (a formula reads one as {name}; list_parameters has them all): ${params}. ` +
    `Tabs: ${tabs}. ` +
    `Tiles and the data they currently show (built-in tiles are not editable):\n${tileContent}\n` +
    `When creating a chart use board="${board}". To change a tile you created, use update_chart with its id. ` +
    `You can also explain what the charts show.`

  // The parts that are never cut take this much room; the filter chips and
  // the tile list share what is left.
  const overhead = render("", null, "").length
  const budget = PAGE_CONTEXT_MAX_CHARS - overhead

  let chips = filterChips
  let json = filterJson
  let tileContent = tiles || "none yet"

  // The raw filter JSON goes first: the chip text says the same thing in
  // the words the user sees.
  const whole = render(chips, json, tileContent)
  if (whole.length <= PAGE_CONTEXT_MAX_CHARS) return whole
  json = null
  const withoutJson = render(chips, json, tileContent)
  if (withoutJson.length <= PAGE_CONTEXT_MAX_CHARS) return withoutJson

  // Then the end of the tile list, cut to the room the chips leave. A room
  // too small for the note means the chips are what eat the budget, and
  // they give way instead of the tile list.
  const roomForTiles = budget - chips.length
  if (roomForTiles >= TILE_NOTE.length && tileContent.length > roomForTiles) {
    tileContent = cutWithNote(tileContent, roomForTiles, TILE_NOTE)
    const withFewerTiles = render(chips, json, tileContent)
    if (withFewerTiles.length <= PAGE_CONTEXT_MAX_CHARS) return withFewerTiles
  }

  // Then the filter chip text, cut the same way.
  chips = cutWithNote(chips, Math.max(budget - tileContent.length, FILTER_NOTE.length), FILTER_NOTE)
  const withFewerChips = render(chips, json, tileContent)
  if (withFewerChips.length <= PAGE_CONTEXT_MAX_CHARS) return withFewerChips

  // Both are longer than the budget allows: the tiles give way first and
  // keep only their note, and the chips keep what is left.
  tileContent = TILE_NOTE
  chips = cutWithNote(filterChips, Math.max(budget - TILE_NOTE.length, FILTER_NOTE.length), FILTER_NOTE)
  return render(chips, json, tileContent)
}
