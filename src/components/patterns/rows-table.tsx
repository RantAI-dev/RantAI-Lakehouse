"use client"

import { categoryLabel } from "@/lib/chart-transforms"
import { formatLocaleNumber } from "@/lib/format"

/**
 * A plain scrollable table of result rows.
 *
 * For places that show the numbers behind something — a chart's data, an
 * agent's answer — where the full data table (sorting, filters, column
 * menus) would be more machinery than the moment deserves. Anything the
 * user works *with* rather than glances at belongs in `DataTable`.
 */
export function RowsTable({
  columns,
  labels,
  rows,
  maxRows,
  className,
  namePlaceholders,
  numberColumns,
  onRowClick,
}: {
  readonly columns: readonly string[]
  /** Header text by column name, for a column whose name is not what a reader should see; the name otherwise. */
  readonly labels?: Readonly<Record<string, string>>
  readonly rows: readonly Record<string, unknown>[]
  /** Show at most this many rows; the caller says so if it matters. */
  readonly maxRows?: number
  readonly className?: string
  /**
   * QA `LH-BI-6-38` (`BI-55`, D8): a NULL cell reads "(No value)" and an
   * empty string "(Empty)", in muted text, as a chart names them. Absent, both
   * are blank as before — a file holds values, not display labels, so a CSV
   * of these rows keeps the cell empty.
   */
  readonly namePlaceholders?: boolean
  /**
   * `BI-58` T6: columns whose numbers people read as amounts, shown right
   * aligned with the browser's thousands separators. Only these: a year or
   * an id in another column stays as it is.
   */
  readonly numberColumns?: readonly string[]
  /** Makes each row open something (a one-record view); receives the row's index in `rows`. */
  readonly onRowClick?: (index: number) => void
}) {
  const shown = maxRows == null ? rows : rows.slice(0, maxRows)
  return (
    <div
      className={
        className ??
        "max-h-[60vh] overflow-auto rounded-md border border-border"
      }
    >
      <table className="w-full border-collapse text-xs">
        <thead className="sticky top-0 bg-card">
          <tr className="border-b border-border">
            {columns.map((c) => (
              <th
                key={c}
                className="px-2 py-1.5 text-left font-medium text-muted-foreground"
              >
                {labels?.[c] ?? c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {shown.map((row, i) => (
            <tr
              key={i}
              className={
                onRowClick
                  ? "cursor-pointer border-b border-border/40 last:border-0 hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:outline-none"
                  : "border-b border-border/40 last:border-0"
              }
              tabIndex={onRowClick ? 0 : undefined}
              onClick={onRowClick ? () => onRowClick(i) : undefined}
              onKeyDown={
                onRowClick
                  ? (e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault()
                        onRowClick(i)
                      }
                    }
                  : undefined
              }
            >
              {columns.map((c) => {
                const blank = row[c] === null || row[c] === undefined || row[c] === ""
                const amount = numberColumns?.includes(c) ? asNumber(row[c]) : null
                return (
                  <td
                    key={c}
                    className={`whitespace-nowrap px-2 py-1 tabular-nums${namePlaceholders && blank ? " text-muted-foreground" : ""}${amount !== null ? " text-right" : ""}`}
                  >
                    {namePlaceholders && blank ? categoryLabel(row[c]) : amount !== null ? formatLocaleNumber(amount) : row[c] == null ? "" : String(row[c])}
                  </td>
                )
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/** A cell's number: a number, or the decimal text ClickHouse sends a 64-bit integer as; null otherwise. */
function asNumber(value: unknown): number | null {
  if (typeof value === "number") return Number.isFinite(value) ? value : null
  if (typeof value === "string" && /^-?\d+(\.\d+)?$/.test(value)) return Number(value)
  return null
}
