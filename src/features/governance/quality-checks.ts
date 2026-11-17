// The checks a quality rule can be built from, and the threshold text each
// one is written as. The text is exactly the grammar the API reads
// (`routes::quality::parse_threshold`), so a rule authored from a picker is
// always one that can be run. DATA-14 (F1/F7): the library grew from four
// checks to the spec's set; both pickers (the asset's Quality tab and the
// Data Quality page) build their threshold here, so they cannot drift.

/** What the evaluator can check, and the quality dimension each one is. */
export const KINDS = {
  rows: { label: "Row count is at least…", dimension: "volume", column: false },
  "not-null": { label: "Column is filled in (share of rows)", dimension: "completeness", column: true },
  nulls: { label: "Column has at most… nulls", dimension: "completeness", column: true },
  unique: { label: "Column has no repeated values", dimension: "uniqueness", column: true },
  duplicates: { label: "Column has at most… repeated values", dimension: "uniqueness", column: true },
  distinct: { label: "Column has at least… different values", dimension: "uniqueness", column: true },
  fresh: { label: "Newest value is no older than…", dimension: "timeliness", column: true },
  accepted: { label: "Column only holds accepted values", dimension: "validity", column: true },
  range: { label: "Column stays within a range", dimension: "validity", column: true },
  stat: { label: "Column's minimum, maximum or average stays within a range", dimension: "validity", column: true },
} as const

export type Kind = keyof typeof KINDS

export type NullUnit = "percent" | "count"
export type DurationUnit = "m" | "h" | "d"
export type Stat = "min" | "max" | "avg"

export type RuleForm = {
  kind: Kind
  column: string
  /** `rows`: the minimum row count. */
  rows: string
  /** `not-null`: the minimum share, 0–100. */
  percent: string
  /** `range` and `stat`: either bound may be left empty. */
  min: string
  max: string
  /** `nulls`, `duplicates`, `distinct` (a count) and `fresh` (a duration). */
  amount: string
  /** `nulls`: whether `amount` is a percentage or a count. */
  nullUnit: NullUnit
  /** `fresh`: the unit of `amount`. */
  durationUnit: DurationUnit
  /** `accepted`: one value per line. */
  values: string
  /** `stat`: which statistic the bounds hold. */
  stat: Stat
}

export const EMPTY_FORM: RuleForm = {
  kind: "not-null",
  column: "",
  rows: "1",
  percent: "100",
  min: "",
  max: "",
  amount: "0",
  nullUnit: "percent",
  durationUnit: "h",
  values: "",
  stat: "min",
}

/** The API's cap on an `in (…)` list and on one value of it. */
export const MAX_ACCEPTED_VALUES = 200
export const MAX_ACCEPTED_VALUE_CHARS = 200

const isNumber = (v: string) => v.trim() !== "" && Number.isFinite(Number(v))
const isWhole = (v: string) => /^\d+$/.test(v.trim())

/** The values of an `accepted` form: its non-empty lines, as typed. */
export function acceptedValues(text: string): string[] {
  return text
    .split("\n")
    .map((v) => v.trim())
    .filter((v) => v !== "")
}

function bounds(call: string, min: string, max: string): string | null {
  const [lo, hi] = [min.trim(), max.trim()]
  if ((lo && !isNumber(lo)) || (hi && !isNumber(hi))) return null
  if (lo && hi) return `${call} between ${Number(lo)} and ${Number(hi)}`
  if (lo) return `${call} >= ${Number(lo)}`
  if (hi) return `${call} <= ${Number(hi)}`
  return null
}

/**
 * The threshold the evaluator reads for what the form says, or `null`
 * while the form is incomplete.
 */
export function ruleThreshold(f: RuleForm): string | null {
  const column = f.column.trim()
  if (KINDS[f.kind].column && !column) return null
  switch (f.kind) {
    case "rows":
      return isWhole(f.rows) ? `rows >= ${f.rows.trim()}` : null
    case "not-null": {
      if (!isNumber(f.percent)) return null
      const p = Number(f.percent)
      if (p < 0 || p > 100) return null
      return p === 100 ? `${column} not null` : `${column} not null >= ${p}%`
    }
    case "nulls": {
      if (f.nullUnit === "count") return isWhole(f.amount) ? `${column} nulls <= ${f.amount.trim()}` : null
      if (!isNumber(f.amount)) return null
      const p = Number(f.amount)
      return p >= 0 && p <= 100 ? `${column} nulls <= ${p}%` : null
    }
    case "unique":
      return `${column} unique`
    case "duplicates":
      return isWhole(f.amount) ? `${column} duplicates <= ${f.amount.trim()}` : null
    case "distinct":
      return isWhole(f.amount) ? `${column} distinct >= ${f.amount.trim()}` : null
    case "fresh": {
      // The API takes whole units from 1 up to a million.
      const n = Number(f.amount)
      return isWhole(f.amount) && n >= 1 && n <= 1_000_000
        ? `${column} fresh within ${n}${f.durationUnit}`
        : null
    }
    case "accepted": {
      const values = acceptedValues(f.values)
      if (
        values.length === 0 ||
        values.length > MAX_ACCEPTED_VALUES ||
        values.some((v) => v.length > MAX_ACCEPTED_VALUE_CHARS)
      ) {
        return null
      }
      // A quote inside a value is doubled, as in SQL; the API reads it back.
      return `${column} in (${values.map((v) => `'${v.replaceAll("'", "''")}'`).join(", ")})`
    }
    case "range":
      return bounds(column, f.min, f.max)
    case "stat":
      return bounds(`${f.stat}(${column})`, f.min, f.max)
  }
}

/** A name for a rule built from the form, when the person gives none. */
export function defaultRuleName(table: string, f: RuleForm): string {
  const what: Record<Kind, string> = {
    rows: "row_count",
    "not-null": "not_null",
    nulls: "nulls",
    unique: "unique",
    duplicates: "duplicates",
    distinct: "distinct",
    fresh: "fresh",
    accepted: "accepted_values",
    range: "range",
    stat: f.stat,
  }
  return [table, KINDS[f.kind].column ? f.column : null, what[f.kind]].filter(Boolean).join("_")
}

/** The forms of a typed threshold, for the help under a free-text field. */
export const THRESHOLD_EXAMPLES = [
  "rows >= 1000",
  "column not null >= 95%",
  "column nulls <= 5%",
  "column unique",
  "column duplicates <= 10",
  "column distinct >= 3",
  "column fresh within 6h",
  "column in ('a', 'b')",
  "min(column) >= 0",
  "column between 0 and 100",
] as const
