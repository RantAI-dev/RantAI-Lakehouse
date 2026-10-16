/**
 * Calculated fields (BI-8, with AI-4): a named formula on a mart or a SQL
 * source that a chart picks like a column. Mirrors
 * `rust/crates/lakehouse-bi/src/fields.rs` and the routes in
 * `rust/crates/lakehouse-api/src/routes/calc_fields.rs`.
 *
 * The formula language is parsed and compiled by the server; the console
 * never builds SQL from it. Reading and checking need `dashboard:read`,
 * saving, changing and deleting `dashboard:write`.
 */

/** Which source a field belongs to: exactly one of the two. */
export type CalcFieldSource = { mart: string; source?: undefined } | { source: string; mart?: undefined }

/**
 * `row`: one value per row (a dimension, or aggregated by the chart).
 * `aggregate`: a measure on its own. `table`: a table calculation or period
 * comparison, computed over the chart's grouped result (a measure of a bar,
 * line, area, stacked, combo, waterfall or grouped table chart only).
 */
export type CalcFieldLevel = "row" | "aggregate" | "group" | string

export type CalcField = {
  id: string
  sourceKind: "mart" | "sql_source" | string
  sourceId: string
  name: string
  formula: string
  level: CalcFieldLevel
  /** `number`, `text`, `date`, `datetime` or `boolean`. */
  type: string
  createdBy: string
  updatedAt?: string
}

/**
 * A value group (BI-10): chosen values of a text column gathered under names of
 * the person's own, with the rest kept as they are or called "Other". It is
 * saved with the calculated fields of its source (`level: "group"`, its
 * definition as JSON in `formula`), picked like a text column, and not usable
 * inside a formula.
 */
export type ValueGroup = { name: string; values: string[] }
export type GroupDef = { column: string; groups: ValueGroup[]; rest: "keep" | "other" }

/** The limits the server enforces (`lakehouse_bi::groups`). */
export const MAX_GROUPS = 50
export const MAX_GROUP_VALUES = 2000

/** Whether a field is a value group. */
export function isValueGroup(field: Pick<CalcField, "level">): boolean {
  return field.level === "group"
}

/** A group field's definition, or null when its text is not one. */
export function groupDefOf(field: Pick<CalcField, "formula">): GroupDef | null {
  try {
    const d = JSON.parse(field.formula) as Partial<GroupDef>
    if (typeof d.column !== "string" || !Array.isArray(d.groups) || (d.rest !== "keep" && d.rest !== "other")) return null
    return { column: d.column, rest: d.rest, groups: d.groups.map((g) => ({ name: String(g.name), values: (g.values ?? []).map(String) })) }
  } catch {
    return null
  }
}

/** What is wrong in a formula and where; both count characters from 0. */
export type FormulaProblem = { message: string; position: number; length: number }

export type FormulaCheck =
  | { ok: true; level: CalcFieldLevel; type: string }
  | { ok: false; error: FormulaProblem }

/** One entry of the function catalog (the server's single list). */
export type FormulaFunction = {
  name: string
  category: string
  signature: string
  help: string
  example: string
  minArgs: number
  maxArgs: number | null
  aggregate: boolean
  /** A table calculation or period comparison: usable only as a chart measure. */
  table: boolean
}

export type CalcFieldService = {
  list(source: CalcFieldSource, signal?: AbortSignal): Promise<CalcField[]>
  /** Checks only; writes nothing. A mistake in the formula is a normal `ok: false`. */
  validate(source: CalcFieldSource, formula: string, name?: string, signal?: AbortSignal): Promise<FormulaCheck>
  /** A refusal (a name in use, a formula that does not check out) arrives as `invalid_request` with the server's sentence. */
  create(source: CalcFieldSource, name: string, formula: string, signal?: AbortSignal): Promise<CalcField>
  update(id: string, formula: string, signal?: AbortSignal): Promise<CalcField>
  /** BI-10: save a value group on a source; the same name rules and permissions as a formula. */
  createGroup(source: CalcFieldSource, name: string, group: GroupDef, signal?: AbortSignal): Promise<CalcField>
  updateGroup(id: string, group: GroupDef, signal?: AbortSignal): Promise<CalcField>
  /** 409 (as `invalid_request`) names the charts or fields still using it. */
  remove(id: string, signal?: AbortSignal): Promise<void>
  functions(signal?: AbortSignal): Promise<FormulaFunction[]>
}
