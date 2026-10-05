/**
 * Pure rules behind the chart builder's inline "custom SQL" panel: when a
 * run still describes the SQL in the editor, what the column pickers are
 * offered, what the source is called, how many other charts an edit would
 * change, and what blocks the save. The panel and its hook only wire these
 * to state, so every decision here is tested without a browser.
 */
import { hasStatement } from "./sql-text"

/** `Select` value of the "Write custom SQL…" item; `decodeSourceChoice` reads it as no choice. */
export const NEW_SQL_CHOICE = "__new_sql__"

/** `TITLE_MAX_CHARS` in `routes/dashboard_sources.rs`. */
export const SOURCE_NAME_MAX_CHARS = 200

type Column = { name: string; type: string }

/**
 * Mirrors `is_numeric_type` in `routes/support.rs` (`Int`, `Float`, `Decimal`
 * anywhere in the type, so `Nullable(Int32)` counts), which is how the API
 * splits a SQL source's columns for `/api/dashboard/fields?source=`. The
 * pickers must offer the same split before the source is saved as after.
 */
const NUMERIC_TYPE_MARKERS = ["Int", "Float", "Decimal"]

export function isNumericColumnType(type: string): boolean {
  return NUMERIC_TYPE_MARKERS.some((m) => type.includes(m))
}

/** Dimension and measure pickers for the columns a preview returned. */
export function fieldsFromColumns(columns: readonly Column[]): {
  dimensions: string[]
  measures: string[]
} {
  const dimensions: string[] = []
  const measures: string[] = []
  for (const c of columns) (isNumericColumnType(c.type) ? measures : dimensions).push(c.name)
  return { dimensions, measures }
}

/** `value` when the picker still offers it, else empty: a column a re-run dropped is not kept. */
export function keepIfOffered(value: string, offered: readonly string[]): string {
  return offered.includes(value) ? value : ""
}

export type SqlRunPhase = "empty" | "unrun" | "running" | "fresh" | "failed" | "stale"

/**
 * What the last run says about the SQL in the editor now. A result or an
 * error belongs to the text that was run: once the text differs, a success
 * is `stale` (its columns may no longer exist) and an error is `unrun` (it
 * described text that is gone).
 */
export function sqlRunPhase(input: {
  act: "idle" | "pending" | "success" | "error"
  ranSql: string | null
  sql: string
}): SqlRunPhase {
  if (!hasStatement(input.sql)) return "empty"
  if (input.act === "pending") return "running"
  if (input.act === "idle" || input.ranSql === null) return "unrun"
  const same = input.ranSql.trim() === input.sql.trim()
  if (input.act === "success") return same ? "fresh" : "stale"
  return same ? "failed" : "unrun"
}

/**
 * The source's name: what the user typed, else the fallback (the chart
 * title for a new source, the saved title for an edit). Typed-and-cleared
 * stays typed (empty), so it is reported as required instead of silently
 * refilling while the user retypes.
 */
export function sourceNameFor(typed: string | null, fallback: string): string {
  return typed ?? fallback
}

/** Why `name` cannot be saved, or `null`. */
export function sourceNameProblem(name: string): string | null {
  const trimmed = name.trim()
  if (!trimmed) return "Source name is required."
  if (Array.from(trimmed).length > SOURCE_NAME_MAX_CHARS) {
    return `Source name is limited to ${SOURCE_NAME_MAX_CHARS} characters.`
  }
  return null
}

type StoredChart = {
  id?: string
  sqlSource?: string | null
  def?: { sqlSource?: string | null } | null
}

/**
 * How many charts other than `excludeChartId` read `sourceId`, from a list of
 * every stored chart (`GET /api/dashboard/specs`, the list the API's own
 * delete guard counts from). A chart is counted once however often it is listed.
 */
export function otherChartsUsing(
  charts: readonly StoredChart[],
  sourceId: string,
  excludeChartId?: string
): number {
  const ids = new Set<string>()
  let anonymous = 0
  for (const c of charts) {
    if ((c.def?.sqlSource ?? c.sqlSource) !== sourceId) continue
    if (c.id === undefined) anonymous += 1
    else if (c.id !== excludeChartId) ids.add(c.id)
  }
  return ids.size + anonymous
}

export function sqlChanged(sql: string, originalSql: string): boolean {
  return sql.trim() !== originalSql.trim()
}

/**
 * Why the chart cannot be saved yet, or `null`. A changed or new SQL must
 * have been run, unchanged since: the column pickers and the saved chart
 * would otherwise point at columns the SQL may no longer return.
 */
export function saveBlocker(input: {
  mode: "new" | "edit"
  sql: string
  originalSql: string
  phase: SqlRunPhase
  name: string
}): string | null {
  const nameProblem = sourceNameProblem(input.name)
  if (input.phase === "empty") return "Write the SQL first."
  if (input.phase === "running") return "Wait for the SQL run to finish."
  const needsRun = input.mode === "new" || sqlChanged(input.sql, input.originalSql)
  if (needsRun && input.phase !== "fresh") {
    if (input.phase === "stale") return "The SQL changed since it was run. Run the SQL again before saving."
    if (input.phase === "failed") return "The SQL did not run. Fix it and run it again before saving."
    return "Run the SQL before saving."
  }
  return nameProblem
}

/** A preview cell as text; `null` is shown as NULL, not as an empty string. */
export function formatCell(value: unknown): string {
  if (value === null || value === undefined) return "NULL"
  if (typeof value === "object") return JSON.stringify(value)
  return String(value)
}
