/**
 * The chart builder's single "Data source" picker holds either a Gold mart
 * or a dashboard SQL source. This module is the one encoding of that choice
 * (`mart:<name>` / `sql:<id>`) and its mapping onto the API's `ChartInput`,
 * where exactly one of `mart` / `sqlSource` may be set
 * (`lakehouse_bi::store::resolve_relation` refuses both).
 */

export type ChartSourceChoice =
  | { kind: "mart"; name: string }
  | { kind: "sql"; id: string }

const MART = "mart:"
const SQL = "sql:"

/** `Select` value for a choice. */
export function encodeSourceChoice(choice: ChartSourceChoice): string {
  return choice.kind === "mart" ? `${MART}${choice.name}` : `${SQL}${choice.id}`
}

/** Parse a `Select` value; `null` for empty or anything unrecognized. */
export function decodeSourceChoice(value: string | null | undefined): ChartSourceChoice | null {
  if (!value) return null
  if (value.startsWith(MART) && value.length > MART.length) {
    return { kind: "mart", name: value.slice(MART.length) }
  }
  if (value.startsWith(SQL) && value.length > SQL.length) {
    return { kind: "sql", id: value.slice(SQL.length) }
  }
  return null
}

/** The choice a stored chart definition was built from, as a `Select` value. */
export function sourceValueFromDef(def: { mart?: string; sqlSource?: string }): string {
  if (def.sqlSource) return encodeSourceChoice({ kind: "sql", id: def.sqlSource })
  if (def.mart) return encodeSourceChoice({ kind: "mart", name: def.mart })
  return ""
}

/** The `ChartInput` fields for a choice — never both. */
export function sourcePayload(
  choice: ChartSourceChoice | null
): { mart: string } | { sqlSource: string } | Record<string, never> {
  if (!choice) return {}
  return choice.kind === "mart" ? { mart: choice.name } : { sqlSource: choice.id }
}

/** `/api/dashboard/fields` query for a choice's columns. */
export function fieldsQuery(choice: ChartSourceChoice): string {
  return choice.kind === "mart"
    ? `mart=${encodeURIComponent(choice.name)}`
    : `source=${encodeURIComponent(choice.id)}`
}
