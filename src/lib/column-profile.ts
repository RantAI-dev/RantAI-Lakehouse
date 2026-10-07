/**
 * The arithmetic behind the Schema tab's column explorer: what share of the
 * rows each listed value, the nulls and the rest take, and which columns a
 * table's sorting key names.
 *
 * Shares are of the rows profiled (`AssetProfile.rowsProfiled`), the number
 * the note under the table names. Only what the profile states is drawn:
 * `GET /api/catalog/{id}/profile` lists at most five values and only where
 * the count is exact (`exact_top_values`), so "other" is whatever is left,
 * never a figure the profile gave.
 */
import { formatPercent } from "@/lib/format"
import type { ColumnProfile } from "@/services/contracts/assets"

/** One listed value with its count and its share of the rows profiled. */
export type ValueShare = { value: string; count: number; share: number }

/**
 * A column's rows, split by what they hold. `values` keep the profile's
 * order, which is most frequent first: the route's `approx_top_k` answers
 * in that order (ClickHouse documents it) and `exact_top_values` only
 * filters it. `otherShare` is what neither a listed value nor a null takes;
 * both it and `nullShare` are `null` when the profile gave no null count,
 * since "other" would then include nulls it cannot tell apart.
 */
export type ValueShares = {
  values: ValueShare[]
  otherShare: number | null
  otherCount: number | null
  nullShare: number | null
  nullCount: number | null
}

/** Below this a leftover is floating-point noise from adding fractions, not rows. */
const NOISE = 1e-9

const clamp01 = (n: number) => Math.min(Math.max(n, 0), 1)

/** `share`, cut to the `room` left on the track; a hair over by float noise is not cut. */
const withinRoom = (share: number, room: number) =>
  share > room + NOISE ? Math.max(0, room) : clamp01(share)

/**
 * Splits a profiled column's rows into its listed values, the rest and the
 * nulls. Every share is from 0 to 1 and together they never pass 1: a
 * count above the rows profiled (or one that would take the whole past the
 * end of the track) is cut to what room is left, never drawn beyond it.
 *
 * `null` when there is nothing to split: the column was not profiled or is
 * missing from the profile (`undefined`), or no rows were profiled.
 */
export function valueShares(
  column: ColumnProfile | undefined,
  rowsProfiled: number
): ValueShares | null {
  if (!column?.profiled) return null
  if (!Number.isFinite(rowsProfiled) || rowsProfiled <= 0) return null

  let used = 0
  const values: ValueShare[] = []
  for (const t of column.topValues ?? []) {
    if (!Number.isFinite(t.count) || t.count < 0) continue
    const share = withinRoom(t.count / rowsProfiled, 1 - used)
    used += share
    values.push({ value: t.value, count: t.count, share })
  }

  const stated =
    typeof column.nullFraction === "number" && Number.isFinite(column.nullFraction)
      ? column.nullFraction
      : typeof column.nullCount === "number" && Number.isFinite(column.nullCount)
        ? column.nullCount / rowsProfiled
        : null
  if (stated === null) {
    return { values, otherShare: null, otherCount: null, nullShare: null, nullCount: null }
  }

  const nullShare = withinRoom(stated, 1 - used)
  const left = 1 - used - nullShare
  const otherShare = left < NOISE ? 0 : left
  return {
    values,
    otherShare,
    otherCount: Math.round(otherShare * rowsProfiled),
    nullShare,
    nullCount:
      typeof column.nullCount === "number" && Number.isFinite(column.nullCount)
        ? column.nullCount
        : Math.round(nullShare * rowsProfiled),
  }
}

/** What a profiled column with no listed value is, as far as the profile's numbers show. */
export type NoValuesLabel = "All null" | "Mostly unique" | "Many distinct values"

/** Less than half a row left after taking the nulls off is rounding of `nullFraction * rows`, not a row. */
const NOISE_ROWS = 0.5

/** `distinctCount` is approximate (`uniq`), so "mostly unique" starts a tenth short of every row being different. */
const MOSTLY_UNIQUE = 0.9

/**
 * What to say of a column the profile listed no value for. An empty list is
 * not "unique": the route lists a value only where its count is exact, which
 * holds while the column has at most `TOP_K_RESERVED` (100) distinct values
 * (`catalog_profile.rs`), so, for a column that is not all null, empty means
 * "more distinct values than the profile counts exactly". What the numbers do
 * support:
 *
 * - every row is null: "All null";
 * - the distinct count is at least 90% of the non-null rows: "Mostly unique"
 *   (true then, and approximate, hence "mostly");
 * - otherwise, or with no distinct count: "Many distinct values".
 *
 * Without a null count the non-null rows are taken as all of them, which can
 * only make "Mostly unique" harder to reach, never wrong. `null` when there is
 * nothing to say: the column was not profiled, or no rows were.
 */
export function noValuesLabel(
  column: ColumnProfile | undefined,
  rowsProfiled: number
): NoValuesLabel | null {
  if (!column?.profiled) return null
  if (!Number.isFinite(rowsProfiled) || rowsProfiled <= 0) return null

  const nulls =
    typeof column.nullCount === "number" && Number.isFinite(column.nullCount)
      ? column.nullCount
      : typeof column.nullFraction === "number" && Number.isFinite(column.nullFraction)
        ? column.nullFraction * rowsProfiled
        : null
  const nonNull = rowsProfiled - Math.min(Math.max(nulls ?? 0, 0), rowsProfiled)
  if (nulls !== null && nonNull < NOISE_ROWS) return "All null"

  const distinct = column.distinctCount
  if (typeof distinct === "number" && Number.isFinite(distinct) && distinct >= MOSTLY_UNIQUE * nonNull) {
    return "Mostly unique"
  }
  return "Many distinct values"
}

/**
 * A share as the page writes it, for a value and for the nulls alike: one
 * decimal, except that a share too small to show is "<0.1%" rather than
 * "0.0%", which would say the value is absent. Exactly zero is "0.0%".
 */
export function formatShare(share: number): string {
  return share > 0 && share < 0.001 ? "<0.1%" : formatPercent(share)
}

/** The parts of a sorting key between its top-level commas: `f(a, b), c` is two. */
function topLevelParts(key: string): string[] {
  const parts: string[] = []
  let depth = 0
  let quote: string | null = null
  let start = 0
  for (let i = 0; i < key.length; i++) {
    const ch = key[i]
    if (quote !== null) {
      if (ch === quote) quote = null
    } else if (ch === "'" || ch === '"' || ch === "`") {
      quote = ch
    } else if (ch === "(" || ch === "[") {
      depth += 1
    } else if (ch === ")" || ch === "]") {
      depth = Math.max(0, depth - 1)
    } else if (ch === "," && depth === 0) {
      parts.push(key.slice(start, i))
      start = i + 1
    }
  }
  parts.push(key.slice(start))
  return parts
}

/**
 * The columns a table's sorting key names, as a set of names. A part of
 * the key marks a column only when it is exactly that column's name
 * (backticks or double quotes around it are not part of the name); an
 * expression such as `toYYYYMM(d)` marks nothing, so the page never says a
 * column is a sort key when only a function of it is. Commas inside an
 * expression do not split it, so `cityHash64(a, b), c` marks `c` and not
 * `b`.
 */
export function sortKeyColumns(
  sortingKey: string | null | undefined,
  columnNames: Iterable<string>
): Set<string> {
  const named = new Set(columnNames)
  const marked = new Set<string>()
  if (!sortingKey) return marked
  for (const part of topLevelParts(sortingKey)) {
    const trimmed = part.trim()
    const name =
      trimmed.length >= 2 && /^(`.*`|".*")$/.test(trimmed) ? trimmed.slice(1, -1) : trimmed
    if (name !== "" && named.has(name)) marked.add(name)
  }
  return marked
}
