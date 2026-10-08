"use client"

/**
 * The Check step's preview: the first rows of the file as the API parsed them,
 * in a frame of their own. Same approach as the Sample tab's grid
 * (`features/catalog/sample-grid.tsx`), without its cell picking or sorting,
 * and not its component, which is tied to asset data.
 *
 * - The frame has `isolate`, so the sticky cells' z-indexes stay inside it and
 *   never paint over the page's own sticky bars; it has `min-w-0` and the
 *   table sits in an inner scroller capped in height, so a wide file scrolls
 *   in the frame and never widens the page.
 * - The header row sticks to the top and the row-number gutter to the left.
 *   Sticky cells need an opaque background, so each paints a theme token
 *   (`--muted`, `--card`; opaque in both themes) and the amber tint of a
 *   marked row is a `color-mix` over `--card`, not an alpha. `border-separate`
 *   because the borders of a sticky cell stay behind in a collapsed table.
 * - An empty cell is `(empty)` in the quiet italic the Sample tab uses; a cell
 *   a short row does not have is a quiet dash. A row with MORE cells than the
 *   header is marked in the gutter and named in a line above the frame, and
 *   its extra cells are not drawn: no column without a name is invented. The
 *   numbers are places among the rows shown, not line numbers of the file.
 * - For a Parquet file each header cell carries a second, quiet line "was:
 *   <type>", the type the file declares (the table's column is text), and
 *   the line above the frame counts against the row total the file states.
 */
import { TriangleAlertIcon } from "lucide-react"
import { formatNumber } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { UploadPreview } from "@/services/contracts/uploads"
import { UploadNotice } from "./upload-parts"

const EMPTY_LABEL = "(empty)"
const quiet = "italic text-muted-foreground"
/** Titles for a value cut by the column's width start at this length. */
const TITLE_ABOVE = 24
/** How many marked rows are named before "and more". */
const NAMED_ROWS = 5

const LONG_ROW_TINT = "bg-[color-mix(in_oklab,var(--color-amber-500)_14%,var(--card))]"

/** The 1-based places, among the rows shown, of the rows with more cells than the header has. */
export function longRows(preview: Pick<UploadPreview, "columns" | "rows">): number[] {
  const places: number[] = []
  preview.rows.forEach((row, i) => {
    if (row.length > preview.columns.length) places.push(i + 1)
  })
  return places
}

function plural(n: number, one: string, many: string) {
  return `${formatNumber(n)} ${n === 1 ? one : many}`
}

export function UploadPreviewGrid({ preview, stale }: { readonly preview: UploadPreview; readonly stale: boolean }) {
  const { columns, rows, truncated, parquet } = preview
  // The declared types, by position; a name that does not line up (cannot
  // happen for the API's answer) simply has no line.
  const types = parquet ? columns.map((_, i) => parquet.columns[i]?.type) : null
  const long = longRows(preview)
  const named = long.slice(0, NAMED_ROWS).join(", ") + (long.length > NAMED_ROWS ? " and more" : "")

  return (
    <div className={cn("min-w-0 space-y-2", stale && "opacity-60")}>
      <p className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
        <span className="font-medium text-foreground">{plural(columns.length, "column", "columns")}</span>
        <span aria-hidden>·</span>
        <span>
          {rows.length === 0
            ? "There are no rows below the header row."
            : parquet && parquet.rows > rows.length
              ? `Showing the first ${formatNumber(rows.length)} rows of ${formatNumber(parquet.rows)}.`
              : truncated
                ? `Showing the first ${rows.length} rows. The file has more.`
                : `Showing all ${rows.length} rows.`}
        </span>
      </p>
      {long.length > 0 ? (
        <UploadNotice tone="warning">
          {long.length === 1 ? `Row ${named} has` : `Rows ${named} have`} more cells than the header ({columns.length}).
          The extra cells are not shown here.
        </UploadNotice>
      ) : null}
      <div className="isolate min-w-0 overflow-hidden rounded-lg border border-border">
          <div
            // A scroll area must be reachable from the keyboard to be scrolled.
            tabIndex={0}
            role="region"
            aria-label="Preview of the first rows"
            className={cn(
              "max-h-[22rem] scroll-pl-14 overflow-auto outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50",
              types ? "scroll-pt-12" : "scroll-pt-9"
            )}
          >
            <table className="w-max min-w-full border-separate border-spacing-0 text-left text-xs">
              <thead>
                <tr>
                  <th
                    scope="col"
                    className={cn(
                      "sticky top-0 left-0 z-30 w-14 min-w-14 max-w-14 border-r border-b border-border bg-muted px-2 text-right align-middle text-[11px] font-medium text-muted-foreground",
                      types ? "h-12" : "h-9"
                    )}
                  >
                    <span className="sr-only">Row number</span>
                    <span aria-hidden>#</span>
                  </th>
                  {columns.map((name, i) => (
                    <th
                      key={i}
                      scope="col"
                      className={cn(
                        "sticky top-0 z-20 border-b border-border bg-muted px-2 align-middle font-medium",
                        types ? "h-12" : "h-9"
                      )}
                    >
                      <div title={name.length > TITLE_ABOVE ? name : undefined} className="max-w-64 truncate font-mono">
                        {name === "" ? <span className={quiet}>{EMPTY_LABEL}</span> : name}
                      </div>
                      {types?.[i] !== undefined ? (
                        <div className="max-w-64 truncate text-[11px] font-normal text-muted-foreground">
                          was: {types[i]}
                        </div>
                      ) : null}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody className="[&>tr:last-child>*]:border-b-0">
                {rows.map((row, r) => {
                  const isLong = row.length > columns.length
                  return (
                    <tr key={r} className="group/row">
                      <th
                        scope="row"
                        className={cn(
                          "sticky left-0 z-10 w-14 min-w-14 max-w-14 border-r border-b border-border px-2 py-1.5 text-right align-middle text-[11px] font-normal tabular-nums text-muted-foreground",
                          isLong ? LONG_ROW_TINT : "bg-card group-hover/row:bg-muted"
                        )}
                      >
                        <span className="flex items-center justify-end gap-1">
                          {isLong ? (
                            <span
                              role="img"
                              aria-label={`This row has ${row.length} cells; the header has ${columns.length}`}
                              title={`This row has ${row.length} cells; the header has ${columns.length}. The extra cells are not shown.`}
                            >
                              <TriangleAlertIcon className="size-3 text-amber-600 dark:text-amber-400" aria-hidden />
                            </span>
                          ) : null}
                          {r + 1}
                        </span>
                      </th>
                      {columns.map((_, c) => {
                        const cell = row[c]
                        return (
                          <td
                            key={c}
                            className={cn(
                              "border-b border-border px-2 py-1.5 align-middle",
                              isLong ? LONG_ROW_TINT : "bg-card group-hover/row:bg-muted"
                            )}
                          >
                            <div
                              title={cell === undefined ? "No cell in this row" : cell.length > TITLE_ABOVE ? cell : undefined}
                              className="max-w-64 truncate font-mono"
                            >
                              {cell === undefined ? (
                                <span className={quiet}>—</span>
                              ) : cell === "" ? (
                                <span className={quiet}>{EMPTY_LABEL}</span>
                              ) : (
                                cell
                              )}
                            </div>
                          </td>
                        )
                      })}
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </div>
      </div>
    </div>
  )
}
