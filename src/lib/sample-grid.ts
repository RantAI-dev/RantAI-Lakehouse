/**
 * The arithmetic behind the Sample tab's grid: what a cell holds, and how
 * the rows on screen are put in order.
 *
 * The sample's rows are strings as stored, `null` for a `NULL`
 * (`AssetDetail.sample`). Nothing here reformats a value: an id like
 * `0250161` is shown and compared as it is stored.
 *
 * Sorting is of the rows the grid holds, never of the table: a sort asks the
 * API for nothing, so on a table of a million rows it orders the first 25.
 * The tab says so next to the sort.
 */
import type { TypeFamily } from "@/lib/column-type"

/**
 * What a cell holds. `NULL` (no value) and an empty text are different
 * things; a cell that a row does not carry at all has no value either.
 */
export type CellKind = "null" | "empty" | "value"

export function cellKind(value: string | null | undefined): CellKind {
  if (value === null || value === undefined) return "null"
  return value === "" ? "empty" : "value"
}

export type SortDirection = "asc" | "desc"

/** The column the rows are ordered by and which way; `null` is the table's own order. */
export type SortState = { column: string; direction: SortDirection } | null

/**
 * What pressing a column's sort control does: none, then ascending, then
 * descending, then back to none. A different column starts at ascending,
 * whatever the one before was doing.
 */
export function nextSort(current: SortState, column: string): SortState {
  if (current === null || current.column !== column) return { column, direction: "asc" }
  return current.direction === "asc" ? { column, direction: "desc" } : null
}

const DECIMAL = /^[+-]?(\d+(\.\d*)?|\.\d+)([eE][+-]?\d+)?$/
const INTEGER = /^[+-]?\d+$/

/**
 * Compared as numbers: plain integers exactly (a 64-bit id past 2^53 would
 * lose its last digits as a double and tie with its neighbour), anything
 * else through `Number`.
 */
function compareNumbers(a: string, b: string): number {
  if (INTEGER.test(a) && INTEGER.test(b)) {
    const x = BigInt(a)
    const y = BigInt(b)
    return x < y ? -1 : x > y ? 1 : 0
  }
  const x = Number(a)
  const y = Number(b)
  return x < y ? -1 : x > y ? 1 : 0
}

// One collator, so text sorts the way the reader's language sorts it
// ("apple" before "Banana") and the cost of building it is paid once.
const collator = new Intl.Collator()

/**
 * `a` against `b`, both holding a value. In a number column the values that
 * read as numbers come first, in numeric order, and the rest after them as
 * text: a pair of one number and one non-number must have an answer, and
 * "numbers first" is the one that keeps the whole order consistent (text
 * against text for such a pair is not: `9 < 10`, `10 < 1e`, `1e < 9`).
 */
function compareValues(a: string, b: string, numeric: boolean): number {
  if (numeric) {
    const aNum = DECIMAL.test(a)
    const bNum = DECIMAL.test(b)
    if (aNum && bNum) return compareNumbers(a, b)
    if (aNum !== bNum) return aNum ? -1 : 1
  }
  return collator.compare(a, b)
}

/**
 * The order `column` gives the rows, as the position each row had in
 * `rows`, first to last. Numbers compare as numbers only when `family` says
 * the column is a number column (a column the schema does not list has no
 * family and sorts as text, never guessed from its values); everything else
 * compares as text. `NULL` is last in both directions: "no value" is not the
 * smallest or the largest. Equal values keep the order they had, also when
 * descending, so the table's order breaks ties.
 *
 * The grid keeps a picked row by its position in the table's own order, so
 * it needs the positions and not only the sorted rows.
 */
export function sortedOrder(
  rows: readonly Record<string, string | null | undefined>[],
  column: string,
  direction: SortDirection,
  family: TypeFamily | undefined
): number[] {
  const numeric = family === "number"
  const sign = direction === "asc" ? 1 : -1
  return rows
    .map((row, at) => ({ at, value: row[column] ?? null }))
    .sort((a, b) => {
      const { value: x } = a
      const { value: y } = b
      if (x === null || y === null) return x === y ? a.at - b.at : x === null ? 1 : -1
      return sign * compareValues(x, y, numeric) || a.at - b.at
    })
    .map(({ at }) => at)
}

/** The rows in the order `sortedOrder` gives, as a new array; the input is not touched. */
export function sortRows<R extends Record<string, string | null | undefined>>(
  rows: readonly R[],
  column: string,
  direction: SortDirection,
  family: TypeFamily | undefined
): R[] {
  return sortedOrder(rows, column, direction, family).map((at) => rows[at])
}
