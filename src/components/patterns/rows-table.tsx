"use client"

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
  rows,
  maxRows,
  className,
}: {
  readonly columns: readonly string[]
  readonly rows: readonly Record<string, unknown>[]
  /** Show at most this many rows; the caller says so if it matters. */
  readonly maxRows?: number
  readonly className?: string
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
                {c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {shown.map((row, i) => (
            <tr key={i} className="border-b border-border/40 last:border-0">
              {columns.map((c) => (
                <td key={c} className="whitespace-nowrap px-2 py-1 tabular-nums">
                  {row[c] == null ? "" : String(row[c])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
