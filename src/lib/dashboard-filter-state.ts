/**
 * The dashboard filter state that lives in the page address (BI-18 part A).
 *
 * Filters are temporary: they are mirrored in the `f` query parameter and
 * only become the dashboard's default when an editor presses "Save as
 * default". Everything here is pure (no fetch, no React) so the rules the
 * page depends on are testable: what counts as an active filter, how a state
 * is encoded and compared with the saved default, and how a chip reads.
 */

import type { FilterDef, FilterKind, FilterOp, RelativeUnit } from "@/services/clients/bi-store"

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
      return Boolean(f.unit) && Boolean(f.anchor) && ((f.anchor !== "last" && f.anchor !== "next") || Boolean(f.n))
    default:
      return Boolean(f.text)
  }
}

/**
 * The filters that restrict something. Applied wherever filters enter the
 * state (the board's saved default, `defaultFilters`, a decoded `?f=`): the
 * pre-BI-18 bar stored a column with an empty selection as `{column, values: []}`,
 * which filters nothing, so it must not occupy the column (no chip, and the
 * column stays in "Add filter"). A required flag on such a placeholder goes
 * with it: a filter that restricts nothing cannot be saved as required, so a
 * stored one is treated as not required.
 */
export function dropInertFilters(filters: FilterDef[]): FilterDef[] {
  return filters.filter(isActiveFilter)
}

/** The filter reduced to the fields its op uses, so equal filters compare equal. */
function canonical(f: FilterDef): FilterDef {
  const op = opOf(f)
  const base = { column: f.column, values: [] as string[] }
  const required = f.required ? { required: true } : {}
  switch (op) {
    case "in":
      return { ...base, values: [...f.values].sort(), ...required }
    case "not_in":
      return { ...base, op, values: [...f.values].sort(), ...required }
    case "between":
      return {
        ...base, op,
        ...(f.min ? { min: f.min } : {}), ...(f.max ? { max: f.max } : {}),
        ...(f.min && f.minExclusive ? { minExclusive: true } : {}),
        ...(f.max && f.maxExclusive ? { maxExclusive: true } : {}),
        ...required,
      }
    case "relative":
      return {
        ...base, op, unit: f.unit, anchor: f.anchor,
        ...(f.anchor === "last" || f.anchor === "next" ? { n: f.n } : {}),
        ...required,
      }
    default:
      return { ...base, op, text: f.text, ...required }
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

const OPS: ReadonlySet<string> = new Set(["in", "not_in", "between", "relative", "contains", "starts_with", "ends_with", "not_contains"])

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
  if (r.minExclusive === true) f.minExclusive = true
  if (r.maxExclusive === true) f.maxExclusive = true
  if (r.required === true) f.required = true
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
  return dropInertFilters(out)
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

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]

/** `2026-10-03` as `3 Oct 2026`; the input unchanged when it is not an ISO date. */
function friendlyDate(iso: string): string {
  const d = parseIso(iso)
  return d ? `${d.day} ${MONTHS[d.month - 1].slice(0, 3)} ${d.year}` : iso
}

function parseIso(iso: string | undefined): { year: number; month: number; day: number } | null {
  if (!iso || !ISO_DATE.test(iso)) return null
  const [year, month, day] = iso.split("-").map(Number)
  if (month < 1 || month > 12 || day < 1 || day > daysIn(year, month)) return null
  return { year, month, day }
}

function daysIn(year: number, month: number): number {
  if (month === 2) return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28
  return [4, 6, 9, 11].includes(month) ? 30 : 31
}

const pad = (n: number, w = 2) => String(n).padStart(w, "0")
const isoDate = (year: number, month: number, day: number) => `${pad(year, 4)}-${pad(month)}-${pad(day)}`

/** Years the API accepts in a date bound (`ClickHouse` `Date32` as the server validates it). */
const MIN_YEAR = 1900
const MAX_YEAR = 2299

/** A whole calendar month or quarter named by a range, or null. Prefer these readings: they are what a person picked. */
function wholePeriod(f: FilterDef): { kind: "month"; year: number; month: number } | { kind: "quarter"; year: number; quarter: number } | null {
  if (f.minExclusive || f.maxExclusive) return null
  const lo = parseIso(f.min)
  const hi = parseIso(f.max)
  if (!lo || !hi || lo.day !== 1 || lo.year !== hi.year) return null
  if (hi.day !== daysIn(hi.year, hi.month)) return null
  if (hi.month === lo.month) return { kind: "month", year: lo.year, month: lo.month }
  if (lo.month % 3 === 1 && hi.month === lo.month + 2) return { kind: "quarter", year: lo.year, quarter: (lo.month - 1) / 3 + 1 }
  return null
}

const numericEqual = (a: string, b: string) => a.trim() !== "" && b.trim() !== "" && Number(a) === Number(b)

function betweenLabel(f: FilterDef): string {
  const c = f.column
  const { min, max } = f
  const lowEx = Boolean(f.minExclusive && min)
  const highEx = Boolean(f.maxExclusive && max)
  const dates = [min, max].some((v) => v && ISO_DATE.test(v))
  if (dates) {
    const period = wholePeriod(f)
    if (period?.kind === "month") return `${c} in ${MONTHS[period.month - 1]} ${period.year}`
    if (period?.kind === "quarter") return `${c} in Q${period.quarter} ${period.year}`
    if (min && max && min === max && !lowEx && !highEx) return `${c} on ${friendlyDate(min)}`
    if (min && !max) return lowEx ? `${c} after ${friendlyDate(min)}` : `${c} from ${min}`
    if (max && !min) return highEx ? `${c} before ${friendlyDate(max)}` : `${c} until ${max}`
    if (!lowEx && !highEx) return `${c} from ${min} to ${max}`
    return `${c} ${lowEx ? "after" : "from"} ${friendlyDate(min ?? "")} ${highEx ? "and before" : "until"} ${friendlyDate(max ?? "")}`
  }
  if (min && max) {
    if (!lowEx && !highEx) return numericEqual(min, max) ? `${c} = ${min}` : `${c} between ${min} and ${max}`
    return `${c} ${lowEx ? ">" : "≥"} ${min} and ${highEx ? "<" : "≤"} ${max}`
  }
  if (min) return `${c} ${lowEx ? ">" : "≥"} ${min}`
  return `${c} ${highEx ? "<" : "≤"} ${max}`
}

/**
 * How the chip reads: `province is Bali, Aceh`, `date in the last 30 days`,
 * `price ≥ 100`, `date after 3 Oct 2026`. The label is derived from the
 * stored filter alone (BI-18 round two: no stored hint), so where two picks
 * serialise the same the more specific reading wins: a whole calendar month
 * reads as the month. `kind` (the column's, when the caller knows it) only
 * separates "is not 5" on a text column from "≠ 5" on a number column.
 */
export function filterLabel(f: FilterDef, kind?: string): string {
  const c = f.column
  switch (opOf(f)) {
    case "in":
      return `${c} is ${list(f.values)}`
    case "not_in":
      if (kind === "number" && f.values.length === 1) return `${c} ≠ ${f.values[0]}`
      return `${c} is not ${list(f.values)}`
    case "between":
      return betweenLabel(f)
    case "relative": {
      const unit = UNIT_WORD[f.unit ?? ""] ?? "period"
      if (f.anchor === "last" || f.anchor === "next") {
        const n = f.n ?? 0
        const word = f.anchor
        return n === 1 ? `${c} in the ${word} ${unit}` : `${c} in the ${word} ${n} ${unit}s`
      }
      return `${c} in ${f.anchor === "previous" ? "the previous" : "this"} ${unit}`
    }
    case "contains":
      return `${c} contains "${clip(f.text ?? "", 30)}"`
    case "starts_with":
      return `${c} starts with "${clip(f.text ?? "", 30)}"`
    case "not_contains":
      return `${c} does not contain "${clip(f.text ?? "", 30)}"`
    default:
      return `${c} ends with "${clip(f.text ?? "", 30)}"`
  }
}

// ---- what the editors offer, and the filter each pick is stored as ----------
//
// The server has no "on", "before", "equal" or "month" ops: they are all a
// `between` with one or two ends (inclusive unless `minExclusive` /
// `maxExclusive`), and "not equal" is `not_in` with one value (BI-18 round
// two, decision R1). These pure functions are the only place that mapping
// lives, in both directions, so the editor, the chip and the tests agree.

export type DatePick =
  | { mode: "last" | "next"; n: number; unit: RelativeUnit }
  | { mode: "this" | "previous"; unit: RelativeUnit }
  | { mode: "on" | "before" | "after"; date: string }
  | { mode: "range"; from?: string; to?: string }
  | { mode: "month"; year: number; month: number }
  | { mode: "quarter"; year: number; quarter: number }

const validYear = (y: number) => Number.isInteger(y) && y >= MIN_YEAR && y <= MAX_YEAR
const validDate = (d: string | undefined): d is string => {
  const p = parseIso(d)
  return p !== null && validYear(p.year)
}

/** The stored filter for a date pick; `null` while the pick is incomplete or out of range. */
export function datePickToFilter(column: string, pick: DatePick): FilterDef | null {
  const base = { column, values: [] as string[] }
  switch (pick.mode) {
    case "last":
    case "next":
      if (!Number.isInteger(pick.n) || pick.n < 1 || pick.n > 3650) return null
      return { ...base, op: "relative", anchor: pick.mode, unit: pick.unit, n: pick.n }
    case "this":
    case "previous":
      return { ...base, op: "relative", anchor: pick.mode, unit: pick.unit }
    case "on":
      return validDate(pick.date) ? { ...base, op: "between", min: pick.date, max: pick.date } : null
    case "after":
      return validDate(pick.date) ? { ...base, op: "between", min: pick.date, minExclusive: true } : null
    case "before":
      return validDate(pick.date) ? { ...base, op: "between", max: pick.date, maxExclusive: true } : null
    case "range": {
      const from = pick.from || undefined
      const to = pick.to || undefined
      if (!from && !to) return null
      if ((from && !validDate(from)) || (to && !validDate(to))) return null
      if (from && to && from > to) return null
      return { ...base, op: "between", ...(from ? { min: from } : {}), ...(to ? { max: to } : {}) }
    }
    case "month": {
      if (!validYear(pick.year) || !Number.isInteger(pick.month) || pick.month < 1 || pick.month > 12) return null
      return {
        ...base, op: "between",
        min: isoDate(pick.year, pick.month, 1), max: isoDate(pick.year, pick.month, daysIn(pick.year, pick.month)),
      }
    }
    case "quarter": {
      if (!validYear(pick.year) || !Number.isInteger(pick.quarter) || pick.quarter < 1 || pick.quarter > 4) return null
      const first = (pick.quarter - 1) * 3 + 1
      return {
        ...base, op: "between",
        min: isoDate(pick.year, first, 1), max: isoDate(pick.year, first + 2, daysIn(pick.year, first + 2)),
      }
    }
  }
}

/** The pick a stored date filter was made from, or `null` when the editor cannot show it as one. Inverse of [`datePickToFilter`]. */
export function filterToDatePick(f: FilterDef): DatePick | null {
  if (opOf(f) === "relative" && f.unit && f.anchor) {
    if ((f.anchor === "last" || f.anchor === "next") && f.n) return { mode: f.anchor, n: f.n, unit: f.unit }
    if (f.anchor === "this" || f.anchor === "previous") return { mode: f.anchor, unit: f.unit }
    return null
  }
  if (opOf(f) !== "between") return null
  const period = wholePeriod(f)
  if (period?.kind === "month") return { mode: "month", year: period.year, month: period.month }
  if (period?.kind === "quarter") return { mode: "quarter", year: period.year, quarter: period.quarter }
  const lowEx = Boolean(f.minExclusive && f.min)
  const highEx = Boolean(f.maxExclusive && f.max)
  if (f.min && f.max) {
    if (lowEx || highEx) return null
    return f.min === f.max ? { mode: "on", date: f.min } : { mode: "range", from: f.min, to: f.max }
  }
  if (f.min) return lowEx ? { mode: "after", date: f.min } : { mode: "range", from: f.min }
  if (f.max) return highEx ? { mode: "before", date: f.max } : { mode: "range", to: f.max }
  return null
}

export type NumberPick =
  | { mode: "eq" | "ne" | "gt" | "lt"; value: string }
  | { mode: "between"; min?: string; max?: string }

/** A finite number written as text, which is what the server accepts as a bound. */
const isNumberText = (v: string | undefined): v is string => v !== undefined && v.trim() !== "" && Number.isFinite(Number(v))

/** The stored filter for a number pick; `null` while it is incomplete or not a number. */
export function numberPickToFilter(column: string, pick: NumberPick): FilterDef | null {
  const base = { column, values: [] as string[] }
  if (pick.mode === "between") {
    const min = pick.min?.trim() || undefined
    const max = pick.max?.trim() || undefined
    if (!min && !max) return null
    if ((min && !isNumberText(min)) || (max && !isNumberText(max))) return null
    if (min && max && Number(min) > Number(max)) return null
    return { ...base, op: "between", ...(min ? { min } : {}), ...(max ? { max } : {}) }
  }
  const value = pick.value.trim()
  if (!isNumberText(value)) return null
  switch (pick.mode) {
    case "eq":
      return { ...base, op: "between", min: value, max: value }
    case "ne":
      return { ...base, op: "not_in", values: [value] }
    case "gt":
      return { ...base, op: "between", min: value, minExclusive: true }
    case "lt":
      return { ...base, op: "between", max: value, maxExclusive: true }
  }
}

/** The pick a stored number filter was made from, or `null` (a value list, say). Inverse of [`numberPickToFilter`]. */
export function filterToNumberPick(f: FilterDef): NumberPick | null {
  if (opOf(f) === "not_in") return f.values.length === 1 && isNumberText(f.values[0]) ? { mode: "ne", value: f.values[0] } : null
  if (opOf(f) !== "between") return null
  const lowEx = Boolean(f.minExclusive && f.min)
  const highEx = Boolean(f.maxExclusive && f.max)
  if (f.min && f.max) {
    if (lowEx || highEx) return null
    return numericEqual(f.min, f.max) ? { mode: "eq", value: f.min } : { mode: "between", min: f.min, max: f.max }
  }
  if (f.min) return lowEx ? { mode: "gt", value: f.min } : { mode: "between", min: f.min }
  if (f.max) return highEx ? { mode: "lt", value: f.max } : { mode: "between", max: f.max }
  return null
}

// ---- required filters (BI-18 part A, second round) --------------------------
//
// `required` lives on the board's saved default and is enforced by the
// console only: the server stores the flag and never reads it, so public
// and embed views (which render the saved default) are unaffected. What
// counts is the *saved* default's flag, so a default that stops requiring a
// column stops enforcing it at once; the flag on a filter in the working
// state is only what "Save as default" will write.

/** Whether the saved default requires a filter on `column`. */
export function isRequiredColumn(column: string, saved: FilterDef[]): boolean {
  return saved.some((f) => f.required === true && f.column === column && isActiveFilter(f))
}

/**
 * The working filters with every required column present: a column the
 * state omits (a link without it, a cleared value list, a removed chip)
 * gets the saved default's filter for it back. A column the state does
 * filter is left as the person set it. Returns the same array when nothing
 * is missing.
 */
export function enforceRequired(filters: FilterDef[], saved: FilterDef[]): FilterDef[] {
  const present = new Set(filters.filter(isActiveFilter).map((f) => f.column))
  const missing = saved.filter((f) => f.required === true && isActiveFilter(f) && !present.has(f.column))
  if (missing.length === 0) return filters
  const restored = new Set(missing.map((f) => f.column))
  return [...filters.filter((f) => !restored.has(f.column)), ...missing]
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
