import { formatNumber } from "@/lib/format"

/**
 * The sentence a cut result carries.
 *
 * The console knows how many rows it got and which limit cut the result, and
 * nothing more: the total the engine produced is not in the response, so the
 * banner never claims to have seen it. `rowCount` is the number of rows the
 * browser got — the rows the grid shows — not the number of rows the query
 * produced.
 */

/**
 * The server's byte cap for one result (`RESULT_BYTE_CAP` in the API's
 * `routes/query.rs`): 10 MB. The console does not set that cap, so the
 * sentence names the limit the server used.
 */
const BYTE_CAP_LABEL = "10 MB"

export function QueryResultBanner({
  rowCount,
  rowLimit,
  cutBy,
}: {
  /** How many rows the browser got. */
  readonly rowCount: number
  /** The Query Studio row limit, for the sentence the row limit needs. */
  readonly rowLimit: number
  /** Which limit cut the result; absent when the server does not say. */
  readonly cutBy?: "rows" | "bytes"
}) {
  const shown = formatNumber(rowCount)
  const sentence =
    cutBy === "rows"
      ? `Showing the first ${shown} rows: the limit is ${formatNumber(rowLimit)} rows.`
      : cutBy === "bytes"
        ? `Showing the first ${shown} rows: the result passed ${BYTE_CAP_LABEL}.`
        : `Showing the first ${shown} rows: the result was cut.`

  return (
    <p className="rounded-md bg-amber-500/5 px-3 py-2 text-xs text-amber-600 dark:text-amber-400">
      {sentence}
    </p>
  )
}
