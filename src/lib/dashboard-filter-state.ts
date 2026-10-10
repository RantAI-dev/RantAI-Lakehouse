/**
 * The dashboard filter state that lives in the page address (BI-18 part A).
 *
 * Filters are temporary: they are mirrored in the `f` query parameter and
 * only become the dashboard's default when an editor presses "Save as
 * default". Everything here is pure (no fetch, no React) so the rules the
 * page depends on are testable: what counts as an active filter, how a state
 * is encoded and compared with the saved default, and how a chip reads.
 */

import type { FilterDef, FilterKind, FilterOp } from "@/services/clients/bi-store"

/** Query parameter holding the encoded filters. */
export const FILTER_PARAM = "f"

/** Same ceiling as the API (`MAX_FILTERS`); a longer address is not worth parsing. */
const MAX_FILTERS = 20
const MAX_VALUES_IN_LABEL = 3
const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/

export function opOf(f: FilterDef): FilterOp {
  return f.op ?? "in"
}

/**
 * Whether the filter restricts anything. An empty value list, a range with
 * neither end and a text match with no text are placeholders the editor may
 * hold; they are neither sent nor shown as chips. Mirrors `FilterDef::is_active`.
 */
export function isActiveFilter(f: FilterDef): boolean {
  switch (opOf(f)) {
    case "in":
    case "not_in":
      return f.values.length > 0
    case "between":
      return Boolean(f.min) || Boolean(f.max)
    case "relative":
      return Boolean(f.unit) && Boolean(f.anchor) && (f.anchor !== "last" || Boolean(f.n))
    default:
      return Boolean(f.text)
  }
}

/** The filter reduced to the fields its op uses, so equal filters compare equal. */
function canonical(f: FilterDef): FilterDef {
  const op = opOf(f)
  const base = { column: f.column, values: [] as string[] }
  switch (op) {
    case "in":
      return { ...base, values: [...f.values].sort() }
    case "not_in":
      return { ...base, op, values: [...f.values].sort() }
    case "between":
      return { ...base, op, ...(f.min ? { min: f.min } : {}), ...(f.max ? { max: f.max } : {}) }
    case "relative":
      return {
        ...base, op, unit: f.unit, anchor: f.anchor,
        ...(f.anchor === "last" ? { n: f.n } : {}),
      }
    default:
      return { ...base, op, text: f.text }
  }
}

/** Active filters only, canonical and in a stable order (column, then op). */
export function normalizeFilters(filters: FilterDef[]): FilterDef[] {
  return filters
    .filter(isActiveFilter)
    .map(canonical)
    .sort((a, b) => a.column.localeCompare(b.column) || opOf(a).localeCompare(opOf(b)))
}

/** Same filters, ignoring order and inactive placeholders. */
export function filtersEqual(a: FilterDef[], b: FilterDef[]): boolean {
  return JSON.stringify(normalizeFilters(a)) === JSON.stringify(normalizeFilters(b))
}

/** The value of the `f` parameter for this state; `""` when nothing is active. */
export function encodeFilters(filters: FilterDef[]): string {
  const active = normalizeFilters(filters)
  return active.length ? JSON.stringify(active) : ""
}

/**
 * What the `f` parameter should be for `next`, given the saved default:
 * `null` (no parameter) when it is the default, otherwise the normalized list
 * as JSON, `"[]"` included. An explicit empty list matters: a dashboard whose
 * default has filters, shown with all of them removed, is not the default.
 */
export function filtersToParam(next: FilterDef[], saved: FilterDef[]): string | null {
  return filtersEqual(next, saved) ? null : JSON.stringify(normalizeFilters(next))
}

const OPS: ReadonlySet<string> = new Set(["in", "not_in", "between", "relative", "contains", "starts_with", "ends_with"])

function asFilter(raw: unknown): FilterDef | null {
  if (typeof raw !== "object" || raw === null) return null
  const r = raw as Record<string, unknown>
  if (typeof r.column !== "string" || !r.column) return null
  if (r.op !== undefined && (typeof r.op !== "string" || !OPS.has(r.op))) return null
  const str = (v: unknown) => (typeof v === "string" ? v : undefined)
  const values = Array.isArray(r.values) ? r.values.filter((v): v is string => typeof v === "string") : []
  const f: FilterDef = { column: r.column, values }
  if (r.op) f.op = r.op as FilterOp
  if (str(r.min) !== undefined) f.min = str(r.min)
  if (str(r.max) !== undefined) f.max = str(r.max)
  if (str(r.unit)) f.unit = r.unit as FilterDef["unit"]
  if (str(r.anchor)) f.anchor = r.anchor as FilterDef["anchor"]
  if (typeof r.n === "number" && Number.isInteger(r.n)) f.n = r.n
  if (str(r.text) !== undefined) f.text = str(r.text)
  return f
}

/**
 * Filters from an `f` parameter. `null` when the text is not a filter list
 * (a hand-edited or truncated link): the page then falls back to the saved
 * default instead of showing a half-understood state. The API validates the
 * values again; this only refuses what cannot be a filter at all.
 */
export function decodeFilters(raw: string | null | undefined): FilterDef[] | null {
  if (!raw) return null
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }
  if (!Array.isArray(parsed) || parsed.length > MAX_FILTERS) return null
  const out: FilterDef[] = []
  for (const item of parsed) {
    const f = asFilter(item)
    if (!f) return null
    out.push(f)
  }
  return out
}

/** The op a new filter on a column of this kind starts with. */
export function defaultOp(kind: FilterKind | string): FilterOp {
  if (kind === "date" || kind === "datetime") return "relative"
  if (kind === "number") return "between"
  return "in"
}

const UNIT_WORD: Record<string, string> = { day: "day", week: "week", month: "month", quarter: "quarter", year: "year" }

function clip(v: string, max = 24): string {
  return v.length > max ? `${v.slice(0, max - 1)}…` : v
}

function list(values: string[]): string {
  const shown = values.slice(0, MAX_VALUES_IN_LABEL).map((v) => clip(v)).join(", ")
  const more = values.length - MAX_VALUES_IN_LABEL
  return more > 0 ? `${shown} +${more}` : shown
}

/** How the chip reads: `province is Bali, Aceh`, `date in the last 30 days`, `price ≥ 100`. */
export function filterLabel(f: FilterDef): string {
  const c = f.column
  switch (opOf(f)) {
    case "in":
      return `${c} is ${list(f.values)}`
    case "not_in":
      return `${c} is not ${list(f.values)}`
    case "between": {
      const { min, max } = f
      const dates = [min, max].some((v) => v && ISO_DATE.test(v))
      if (min && max) return dates ? `${c} from ${min} to ${max}` : `${c} between ${min} and ${max}`
      if (min) return dates ? `${c} from ${min}` : `${c} ≥ ${min}`
      return dates ? `${c} until ${max}` : `${c} ≤ ${max}`
    }
    case "relative": {
      const unit = UNIT_WORD[f.unit ?? ""] ?? "period"
      if (f.anchor === "last") {
        const n = f.n ?? 0
        return n === 1 ? `${c} in the last ${unit}` : `${c} in the last ${n} ${unit}s`
      }
      return `${c} in ${f.anchor === "previous" ? "the previous" : "this"} ${unit}`
    }
    case "contains":
      return `${c} contains "${clip(f.text ?? "", 30)}"`
    case "starts_with":
      return `${c} starts with "${clip(f.text ?? "", 30)}"`
    default:
      return `${c} ends with "${clip(f.text ?? "", 30)}"`
  }
}

/** Flip `value` in an `in` filter on `column` (a click on a chart), dropping the filter when it empties. */
export function toggleValue(filters: FilterDef[], column: string, value: string): FilterDef[] {
  const existing = filters.find((f) => f.column === column && opOf(f) === "in")
  if (!existing) return [...filters, { column, values: [value] }]
  const values = existing.values.includes(value)
    ? existing.values.filter((v) => v !== value)
    : [...existing.values, value]
  return values.length
    ? filters.map((f) => (f === existing ? { ...f, values } : f))
    : filters.filter((f) => f !== existing)
}
