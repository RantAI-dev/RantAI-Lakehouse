/**
 * Parameters (`BI-11`): a named value a viewer changes on a dashboard. The
 * controls live in the filter row, the current values in the address (`?p=`,
 * a JSON object beside `f` and `grain`), and `GET /api/dashboard` takes them as
 * `params`. A formula reads one as `{name}`; a chart's limit, a reference line
 * and a column slot can name one too.
 *
 * In the console a chart's column slot that stands for a parameter is the text
 * `{name}` (never a column name); the server writes it as
 * `{ "param": "name" }` and reads either form. `normalizeDef` turns the
 * server's object form into that text once, where a definition enters the
 * console, so the rest of the code keeps treating those slots as strings.
 */

/** The address parameter, next to the filters' `f` and the grain's `grain`. */
export const PARAMS_PARAM = "p"

export type ParamType = "number" | "text" | "date" | "list"
export type ParamValue = string | number

export type ParamListEntry = { value: ParamValue; label: string }

/** Mirrors `lakehouse_bi::params::ParamDef`. */
export type ParamDef = {
  id: string
  name: string
  label: string
  type: ParamType | string
  default: ParamValue
  min?: ParamValue
  max?: ParamValue
  maxLength?: number
  values?: ParamListEntry[]
  createdBy?: string
  updatedAt?: string
}

/** A control the dashboard payload lists: the definition and the value this read used. */
export type ParamControl = { param: ParamDef; value: ParamValue }

/** What a board saves for a shown parameter. */
export type BoardParam = { name: string; value?: ParamValue }

export const MAX_REFERENCE_LINES = 5

/** `{name}` for a column slot that stands for a parameter. */
export function slotText(name: string): string {
  return `{${name}}`
}

const SLOT = /^\{([A-Za-z_][A-Za-z0-9_]{0,63})\}$/

/** The parameter a slot text names, if it is one. */
export function slotParam(text: string | undefined | null): string | null {
  const m = typeof text === "string" ? SLOT.exec(text) : null
  return m ? m[1] : null
}

type Obj = Record<string, unknown>
const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v)

function slotOf(v: unknown): unknown {
  return isObj(v) && typeof v.param === "string" ? slotText(v.param) : v
}

/**
 * A definition as the console holds it: `{ param }` in `dimension`, `breakdown`,
 * a measure's `column` and `limit` become `{name}` text (a limit stays a
 * `{ param }` object, since it is not a string). Anything else is returned as it is.
 */
export function normalizeDef<T extends object>(def: T | undefined): T | undefined {
  if (!def) return def
  const d = { ...(def as Obj) }
  if ("dimension" in d) d.dimension = slotOf(d.dimension)
  if ("breakdown" in d) d.breakdown = slotOf(d.breakdown)
  if (Array.isArray(d.measures)) {
    d.measures = d.measures.map((m) => {
      if (!isObj(m)) return m
      const column = slotOf(m.column)
      return m.aggregate === undefined && typeof column === "string" && slotParam(column) ? column : { ...m, column }
    })
  }
  return d as T
}

/** Whether a limit is a parameter's: `{ param: name }`. */
export function limitParam(limit: unknown): string | null {
  return isObj(limit) && typeof limit.param === "string" ? limit.param : null
}

/** A `?p=` value as the values it carries; anything that is not an object of text and numbers is no choice. */
export function readParamsParam(raw: string | null): Record<string, ParamValue> | null {
  if (!raw) return null
  try {
    const v: unknown = JSON.parse(raw)
    if (!isObj(v)) return null
    const out: Record<string, ParamValue> = {}
    for (const [k, val] of Object.entries(v)) {
      if (typeof val === "string" || typeof val === "number") out[k] = val
    }
    return Object.keys(out).length ? out : null
  } catch {
    return null
  }
}

/** The `params` to send to the API; absent when the address says nothing. */
export function paramsQuery(values: Record<string, ParamValue> | null): string | null {
  return values && Object.keys(values).length ? JSON.stringify(values) : null
}

/** The address with the values written, or the parameter removed when there are none. */
export function withParamsParam(search: string, values: Record<string, ParamValue> | null): string {
  const q = new URLSearchParams(search)
  const text = paramsQuery(values)
  if (text === null) q.delete(PARAMS_PARAM)
  else q.set(PARAMS_PARAM, text)
  return q.toString()
}

/** One value changed in what the address carries. A value equal to the saved one is dropped, so the address only holds differences. */
export function withValue(
  current: Record<string, ParamValue> | null,
  saved: Record<string, ParamValue>,
  name: string,
  value: ParamValue,
): Record<string, ParamValue> | null {
  const next = { ...(current ?? {}) }
  if (saved[name] !== undefined && String(saved[name]) === String(value)) delete next[name]
  else next[name] = value
  return Object.keys(next).length ? next : null
}

/** Whether the shown values differ from the saved defaults, so "Save as default" is offered. */
export function paramsDiffer(controls: readonly ParamControl[], saved: Record<string, ParamValue> | undefined): boolean {
  return controls.some((c) => String(c.value) !== String(saved?.[c.param.name] ?? c.param.default))
}

/** The `params` of a board PUT: every shown parameter with its current value. */
export function savedParamsBody(controls: readonly ParamControl[]): BoardParam[] {
  return controls.map((c) => ({ name: c.param.name, value: c.value }))
}

/** The values of a read as the builder's preview sends them. */
export function previewValues(applied: Record<string, ParamValue> | undefined): Record<string, ParamValue> | undefined {
  return applied && Object.keys(applied).length ? applied : undefined
}

/** A short rule for the control's hint: the range or the length the definition allows. */
export function paramHint(p: ParamDef): string {
  if (p.type === "number" || p.type === "date") {
    if (p.min !== undefined && p.max !== undefined) return `${p.min} to ${p.max}`
    if (p.min !== undefined) return `at least ${p.min}`
    if (p.max !== undefined) return `at most ${p.max}`
  }
  if (p.type === "text" && p.maxLength) return `up to ${p.maxLength} characters`
  return ""
}

/** The list parameters that can stand for a column: all-text lists. */
export function columnSwitchParams(defs: readonly ParamDef[]): ParamDef[] {
  return defs.filter((d) => d.type === "list" && (d.values ?? []).every((v) => typeof v.value === "string"))
}

/** The number parameters a limit or a reference line can take. */
export function numberParams(defs: readonly ParamDef[]): ParamDef[] {
  return defs.filter((d) => d.type === "number")
}

/** A reference line as the builder holds it. `param` set means the line follows that parameter. */
/** What a reference line marks (`BI-17` part A); mirrors `lakehouse_bi::params::LineKind`. */
export const LINE_KINDS = ["value", "average", "median", "min", "max", "trend", "band", "forecast"] as const
export type LineKind = (typeof LINE_KINDS)[number]

/** Whether the console computes the line from the rows on screen. */
export function isComputedLine(kind: LineKind | string | undefined): boolean {
  return kind === "average" || kind === "median" || kind === "min" || kind === "max" || kind === "trend" || kind === "forecast"
}

/**
 * `BI-17` part B (plan D1): a forecast looks 1 to 60 periods ahead, and a chart
 * has at most one. Mirrors `lakehouse_bi::params` `ReferenceLine.horizon`; the
 * server check lands with T1, so until then these limits are the console's only.
 */
export const MIN_FORECAST_HORIZON = 1
export const MAX_FORECAST_HORIZON = 60
export const DEFAULT_FORECAST_HORIZON = 6

/**
 * A line as the builder edits it. `kind` and the fields after it are
 * `BI-17`; a draft without them is a plain value line.
 */
export type RefLineDraft = {
  value: string
  param: string
  label: string
  kind?: LineKind
  /** A band's upper edge: a number, or a parameter. */
  to?: string
  toParam?: string
  color?: string
  measure?: string
  /** A forecast's periods ahead (`BI-17` part B), as typed. */
  horizon?: string
}

/** A line as the server stores it; `value` is absent on a computed line. */
export type RefLineDef = {
  value?: number | { param: string }
  kind?: LineKind | string
  to?: number | { param: string }
  label?: string
  color?: string
  measure?: string
  /** Periods ahead of a `forecast` line (`BI-17` part B, D1). */
  horizon?: number
}

function edge(v: number | { param: string } | undefined): { text: string; param: string } {
  if (v === undefined) return { text: "", param: "" }
  return typeof v === "number" ? { text: String(v), param: "" } : { text: "", param: v.param }
}

export function refLinesFromDef(lines: readonly RefLineDef[] | undefined): RefLineDraft[] {
  return (lines ?? []).map((l) => {
    const from = edge(l.value)
    const to = edge(l.to)
    return {
      value: from.text,
      param: from.param,
      label: l.label ?? "",
      ...(l.kind && l.kind !== "value" ? { kind: l.kind as LineKind } : {}),
      ...(l.to !== undefined ? { to: to.text, toParam: to.param } : {}),
      ...(l.color ? { color: l.color } : {}),
      ...(l.measure ? { measure: l.measure } : {}),
      ...(l.horizon !== undefined ? { horizon: String(l.horizon) } : {}),
    }
  })
}

function edgePayload(text: string | undefined, param: string | undefined): number | { param: string } | undefined {
  if (param) return { param }
  if (text !== undefined && text.trim() !== "" && Number.isFinite(Number(text))) return Number(text)
  return undefined
}

/**
 * The lines to send; a value or band row with no number or parameter is
 * dropped, and a computed line carries no value.
 */
export function refLinesPayload(rows: readonly RefLineDraft[]): RefLineDef[] {
  const out: RefLineDef[] = []
  let forecasts = 0
  for (const r of rows.slice(0, MAX_REFERENCE_LINES)) {
    const label = r.label.trim()
    const kind = r.kind ?? "value"
    const rest = {
      ...(label ? { label } : {}),
      ...(r.color ? { color: r.color } : {}),
      ...(isComputedLine(kind) && r.measure ? { measure: r.measure } : {}),
    }
    if (kind === "forecast") {
      // At most one per chart, and only with a whole number of periods in range (D1).
      const horizon = Number(r.horizon)
      if (forecasts > 0 || !Number.isInteger(horizon) || horizon < MIN_FORECAST_HORIZON || horizon > MAX_FORECAST_HORIZON) continue
      forecasts += 1
      out.push({ kind, horizon, ...rest })
      continue
    }
    if (isComputedLine(kind)) {
      out.push({ kind, ...rest })
      continue
    }
    const value = edgePayload(r.value, r.param)
    if (value === undefined) continue
    if (kind === "band") {
      const to = edgePayload(r.to, r.toParam)
      if (to !== undefined) out.push({ kind, value, to, ...rest })
      continue
    }
    out.push({ value, ...rest })
  }
  return out
}
