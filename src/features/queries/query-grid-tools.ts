import type { QueryCell } from "@/services/contracts/queries"

/**
 * The arithmetic behind the result grid's tools: the column filters, the
 * column statistics and the histogram.
 *
 * Kept apart from the components so the rules that are easy to get wrong —
 * what counts as an empty cell, when a column counts as numeric, which bin
 * a value falls into — can be tested without rendering a table.
 */

/** One result row: a cell per column, in whatever type the engine reported. */
export type QueryRow = Record<string, QueryCell>

/**
 * A plain number: the same shape the CSV guard calls a number
 * (`PLAIN_NUMBER` in `src/lib/csv.ts`), so "10" is a number while "1,000",
 * "10abc" and "-5 " are text.
 */
const PLAIN_NUMBER = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/

/** How many bins a numeric column is broken into. */
const HISTOGRAM_BINS = 10

/** How many distinct values a text column's histogram shows. */
const TOP_VALUES = 10

/**
 * The text of a cell.
 *
 * A null cell has no text at all, so it never matches a filter that asks for
 * something and never reads as a value in the statistics.
 */
export function cellText(value: QueryCell): string {
  if (value === null) return ""
  if (typeof value === "number") return String(value)
  if (typeof value === "boolean") return String(value)
  return value
}

/**
 * The rows that pass every filter that holds text.
 *
 * A filter matches when the column's cell contains what was typed, whatever
 * the case. Two filters are combined with AND: a row has to pass both to
 * stay. A filter that holds no text excludes nothing, and a null cell passes
 * only such a filter — it has no text to match.
 */
export function filterRows(
  rows: readonly QueryRow[],
  filters: Readonly<Record<string, string>>
): QueryRow[] {
  const active = Object.entries(filters).filter(([, text]) => text !== "")
  if (active.length === 0) return [...rows]
  return rows.filter((row) =>
    active.every(([column, text]) => {
      const cell = row[column]
      const haystack = cellText(cell === undefined ? null : cell).toLowerCase()
      return haystack.includes(text.toLowerCase())
    })
  )
}

/**
 * The number a cell holds, or null when it holds no number.
 *
 * A cell the engine sent as a number is one; a cell that reads as a plain
 * number is one too, because the engine sends every cell as text and a
 * column of numbers only reads as numbers once they are parsed.
 */
function asNumber(value: QueryCell): number | null {
  if (typeof value === "number") return Number.isFinite(value) ? value : null
  if (value === null || typeof value === "boolean") return null
  return PLAIN_NUMBER.test(value) ? Number(value) : null
}

/**
 * Whether a column counts as numeric.
 *
 * Two things make a column numeric: a value the engine sent as a number
 * (the grid's own rule — a numeric column is right-aligned), and a column
 * whose every value reads as a number. The second one matters because the
 * engine sends every cell as text, so without it no column would ever count
 * as numeric and the statistics would always be text order.
 */
function isNumericColumn(values: readonly QueryCell[]): boolean {
  if (values.some((value) => typeof value === "number")) return true
  const readable = values
    .filter((value) => value !== null && cellText(value) !== "")
    .map((value) => asNumber(value))
  return readable.length > 0 && readable.every((value) => value !== null)
}

/** The statistics of one column of the rows the grid shows. */
export type ColumnStatistics = {
  /** How many cells hold nothing: a null or an empty string. */
  empty: number
  /** How many distinct non-empty values the column holds. */
  distinct: number
  /** The smallest value, as text; null when the column holds no value. */
  min: string | null
  /** The largest value, as text; null when the column holds no value. */
  max: string | null
  /** Whether the smallest and the largest were read as numbers. */
  numeric: boolean
}

/**
 * The smallest and the largest of the given numbers.
 *
 * A loop, not `Math.min(...numbers)`: a call that takes one argument per
 * value is refused by some browsers once the column holds more than 65,535
 * rows, and the result grid holds up to 100,000 rows. The calls take two
 * arguments each, so the result stays the one the spread call gave — a
 * signed zero included.
 */
function extremes(numbers: readonly number[]): { min: number; max: number } {
  let min = Number.POSITIVE_INFINITY
  let max = Number.NEGATIVE_INFINITY
  for (const number of numbers) {
    min = Math.min(min, number)
    max = Math.max(max, number)
  }
  return { min, max }
}

/**
 * What one column of the given rows holds: how many cells are empty, how
 * many distinct values there are, and the smallest and the largest value.
 *
 * The smallest and the largest are numbers for a numeric column and text
 * order for a text one. A column that holds no value at all has neither.
 */
export function columnStatistics(
  rows: readonly QueryRow[],
  column: string
): ColumnStatistics {
  const values = rows.map((row) => {
    const cell = row[column]
    return cell === undefined ? null : cell
  })
  const texts = values.map(cellText)
  const empty = texts.filter((text) => text === "").length
  const distinct = new Set(texts.filter((text) => text !== "")).size
  const numeric = isNumericColumn(values)

  if (distinct === 0) {
    return { empty, distinct, min: null, max: null, numeric }
  }

  if (numeric) {
    const numbers = values
      .map(asNumber)
      .filter((value): value is number => value !== null)
    const { min, max } = extremes(numbers)
    return {
      empty,
      distinct,
      min: String(min),
      max: String(max),
      numeric,
    }
  }

  const sorted = [...texts.filter((text) => text !== "")].sort((a, b) =>
    a.localeCompare(b)
  )
  return {
    empty,
    distinct,
    min: sorted[0] ?? null,
    max: sorted[sorted.length - 1] ?? null,
    numeric,
  }
}

/** One bin of a numeric column: the values from `lower` up to `upper`. */
export type HistogramBin = {
  lower: number
  upper: number
  count: number
}

/** One value of a text column and how often it occurs. */
export type HistogramValue = {
  value: string
  count: number
}

/**
 * A column's shape: bins for a numeric column, the most frequent values for
 * a text one. Which shape a column gets follows the same numeric rule as the
 * statistics, so the panel never says "numeric" and draws a text histogram.
 */
export type ColumnHistogram =
  | { kind: "bins"; bins: HistogramBin[] }
  | { kind: "values"; values: HistogramValue[] }

/** The bin a number falls into, its lower edge included and its upper edge not. */
function binIndex(
  value: number,
  min: number,
  width: number,
  binCount: number
): number {
  let index = Math.floor((value - min) / width)
  if (!Number.isFinite(index) || index < 0) index = 0
  if (index > binCount - 1) index = binCount - 1
  // The edges are computed the same way as the index, so a value that sits
  // exactly on an edge lands in the bin that edge opens. Floating point can
  // put it one bin off, which the two loops below correct.
  while (index > 0 && value < min + index * width) index -= 1
  while (index + 1 < binCount && value >= min + (index + 1) * width) index += 1
  return index
}

/**
 * How a column's values are spread out.
 *
 * A numeric column is broken into ten equal-width bins between its smallest
 * and its largest value — fewer bins when the column holds fewer distinct
 * values, and one bin when every value is the same. A text column shows its
 * ten most frequent values, the most frequent first, with the ties broken by
 * the value's own text so the order never moves between two renders.
 */
export function histogram(
  rows: readonly QueryRow[],
  column: string
): ColumnHistogram {
  const values = rows.map((row) => {
    const cell = row[column]
    return cell === undefined ? null : cell
  })
  const texts = values.map(cellText)

  if (!isNumericColumn(values)) {
    const counts = new Map<string, number>()
    for (const text of texts) {
      if (text === "") continue
      counts.set(text, (counts.get(text) ?? 0) + 1)
    }
    const values = [...counts.entries()]
      .map(([value, count]) => ({ value, count }))
      .sort((a, b) =>
        b.count === a.count ? a.value.localeCompare(b.value) : b.count - a.count
      )
      .slice(0, TOP_VALUES)
    return { kind: "values", values }
  }

  const numbers = values
    .map(asNumber)
    .filter((value): value is number => value !== null)
  if (numbers.length === 0) return { kind: "bins", bins: [] }

  const { min, max } = extremes(numbers)
  if (min === max) {
    return { kind: "bins", bins: [{ lower: min, upper: max, count: numbers.length }] }
  }

  const distinct = new Set(numbers).size
  const binCount = Math.min(HISTOGRAM_BINS, distinct)
  const width = (max - min) / binCount
  const bins: HistogramBin[] = Array.from({ length: binCount }, (_, index) => ({
    lower: min + index * width,
    upper: index === binCount - 1 ? max : min + (index + 1) * width,
    count: 0,
  }))
  for (const number of numbers) {
    const index = binIndex(number, min, width, binCount)
    const bin = bins[index]
    if (bin) bin.count += 1
  }
  return { kind: "bins", bins }
}
