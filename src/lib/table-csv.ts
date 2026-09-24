/**
 * Turning what a table currently shows into CSV.
 *
 * Kept apart from the menu that triggers the download so the rules that are
 * easy to get wrong — header names, empty cells, non-string values — can be
 * tested without rendering a table.
 */

import { toCsv } from "./csv"

export type CsvColumn = {
  readonly id: string
  readonly label: string
}

/**
 * Header row, using the labels people already read in the table.
 *
 * Two columns can carry the same label (a page is free to name both a
 * created and an updated column "Date"), and a repeated header makes the
 * file ambiguous, so a duplicate falls back to the column id — unique by
 * construction.
 */
export function csvHeaders(columns: readonly CsvColumn[]): string[] {
  const seen = new Set<string>()
  return columns.map((column) => {
    const header = seen.has(column.label) ? column.id : column.label
    seen.add(header)
    seen.add(column.label)
    return header
  })
}

/**
 * One cell as text. `null` and `undefined` become an empty cell rather than
 * the words "null"/"undefined", dates are written ISO so a spreadsheet can
 * parse them, and anything structured is JSON instead of "[object Object]".
 */
export function csvCell(value: unknown): string {
  if (value == null) return ""
  if (value instanceof Date) return value.toISOString()
  if (typeof value === "object") return JSON.stringify(value)
  return String(value)
}

/**
 * CSV for the given columns and rows, where each row is keyed by column id.
 *
 * Callers pass the rows the table holds right now, so the file matches what
 * is on screen: filtered, sorted, and — on an infinite table — only as far
 * as the user has scrolled.
 */
export function tableCsv(
  columns: readonly CsvColumn[],
  rows: readonly Record<string, unknown>[]
): string {
  const headers = csvHeaders(columns)
  return toCsv(
    headers,
    rows.map((row) =>
      Object.fromEntries(
        columns.map((column, i) => [headers[i], csvCell(row[column.id])])
      )
    )
  )
}

/** "Data Explorer" → "data-explorer-2026-09-20.csv". */
export function csvFileName(name: string, at: Date = new Date()): string {
  const slug =
    name
      .replace(/[^\w\s-]/g, "")
      .trim()
      .replace(/\s+/g, "-")
      .toLowerCase() || "table"
  const day = `${at.getFullYear()}-${String(at.getMonth() + 1).padStart(2, "0")}-${String(at.getDate()).padStart(2, "0")}`
  return `${slug}-${day}.csv`
}
