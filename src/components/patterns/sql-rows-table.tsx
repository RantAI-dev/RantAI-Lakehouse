"use client"

import { formatCell } from "@/lib/sql-source-draft"
import { cn } from "@/lib/utils"

/**
 * The columns and first rows a SQL run returned, shared by the dashboard's
 * SQL source (`features/dashboards/sql-source-panel.tsx`, the chart
 * builder's Preview pane) and the pipeline editor's preview (BLD-1 part 1
 * D17). They moved here from the dashboard panel so two features can use
 * them. Two additions, each defaulting to the dashboard's words: the
 * no-rows sentence (`noRowsText`; a pipeline input is a table, not "the
 * SQL", and `null` on the chips leaves the sentence to the table) and the
 * stale sentence (`staleText`). BLD-1 part 1 S3 review R1-S8.
 *
 * The shape is structural: a dashboard's `SqlSourcePreview` and a pipeline
 * preview turned into records (`lib/pipeline-editor.ts` `previewAsRecords`)
 * both fit it.
 */
export type SqlRowsResult = {
  columns: { name: string; type: string }[]
  rows: Record<string, unknown>[]
}

const NO_ROWS = "The SQL ran and returned no rows."

/**
 * The columns a Run returned, with their types. `noRowsText={null}` leaves
 * the no-rows line to the rows table, where a pane shows both (BLD-1 part 1
 * S3 review R1-S7: the line was said twice).
 */
export function SqlColumnChips({ result, noRowsText = NO_ROWS }: { result: SqlRowsResult; noRowsText?: string | null }) {
  return (
    <div className="grid gap-1.5">
      <div className="flex flex-wrap gap-1.5" aria-label="Columns">
        {result.columns.map((c) => (
          <span key={c.name} className="rounded border border-border px-1.5 py-0.5 font-mono text-[11px]">
            {c.name} <span className="text-muted-foreground">{c.type}</span>
          </span>
        ))}
      </div>
      {result.rows.length === 0 && noRowsText !== null ? <p className="text-xs text-muted-foreground">{noRowsText}</p> : null}
    </div>
  )
}

/**
 * The first rows a Run returned. Lives in the Preview pane only; the panel
 * under the editor lists just the columns, so the rows are never shown twice.
 */
export function SqlRowsTable({
  result,
  stale,
  className,
  noRowsText = NO_ROWS,
  staleText = " These are the rows of the SQL before your last edit.",
}: {
  result: SqlRowsResult
  /** The SQL was edited after this result: say so, since the rows are of the earlier text. */
  stale?: boolean
  className?: string
  noRowsText?: string
  /** Appended to the line above the rows when `stale`. */
  staleText?: string
}) {
  return (
    <div className={cn("grid min-h-0 content-start gap-2", className)}>
      <p className="text-xs text-muted-foreground">
        {result.rows.length === 0
          ? noRowsText
          : `First ${result.rows.length.toLocaleString("en-US")} row${result.rows.length === 1 ? "" : "s"}.`}
        {stale ? staleText : ""}
      </p>
      {result.rows.length ? (
        <div className={cn("min-h-0 overflow-auto rounded-md border border-border", stale && "opacity-60")}>
          <table className="w-full text-left text-xs">
            <thead className="sticky top-0 bg-muted">
              <tr>
                {result.columns.map((c) => (
                  <th key={c.name} scope="col" className="px-2 py-1 font-medium whitespace-nowrap">{c.name}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {result.rows.map((row, i) => (
                <tr key={i} className="border-t border-border">
                  {result.columns.map((c) => {
                    const text = formatCell(row[c.name])
                    return (
                      <td
                        key={c.name} title={text}
                        className={cn("max-w-56 truncate px-2 py-1 font-mono", row[c.name] == null && "text-muted-foreground italic")}
                      >
                        {text}
                      </td>
                    )
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
    </div>
  )
}
