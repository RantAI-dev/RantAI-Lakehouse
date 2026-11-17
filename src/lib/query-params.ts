import type {
  QueryDateRange,
  QueryParamDef,
  QueryParamOption,
  QueryParamType,
  QueryParamValue,
} from "@/services/contracts/queries"

/**
 * Reading `{{name}}` parameter definitions and values from storage
 * (`QS-5`). A tab keeps both in the browser, so what comes back may be
 * old, hand-edited or half written. These readers check the *shape* the
 * wire has and nothing more: the rules the server judges (a real calendar
 * day, the limits, a default that is one of the options) stay the
 * server's, and its sentence is what the person sees when it refuses.
 */

const TYPES: readonly QueryParamType[] = [
  "text",
  "number",
  "date",
  "dateRange",
  "dropdown",
  "multiSelect",
]

export function isQueryParamType(value: unknown): value is QueryParamType {
  return typeof value === "string" && (TYPES as readonly string[]).includes(value)
}

const DAY = /^\d{4}-\d{2}-\d{2}$/

function isDay(value: unknown): value is string {
  return typeof value === "string" && DAY.test(value)
}

/** A list value is a text or a finite number; the server wants all of one list the same kind. */
function isListValue(value: unknown): value is string | number {
  return typeof value === "string" || (typeof value === "number" && Number.isFinite(value))
}

function isDateRange(value: unknown): value is QueryDateRange {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false
  const keys = Object.keys(value)
  if (keys.length !== 2 || !keys.includes("start") || !keys.includes("end")) return false
  const range = value as Record<string, unknown>
  return isDay(range.start) && isDay(range.end)
}

/** Whether `value` is the wire shape of a `type` parameter. */
export function valueFitsType(type: QueryParamType, value: unknown): value is QueryParamValue {
  switch (type) {
    case "text":
      return typeof value === "string"
    case "number":
      return typeof value === "number" && Number.isFinite(value)
    case "date":
      return isDay(value)
    case "dateRange":
      return isDateRange(value)
    case "dropdown":
      return isListValue(value)
    case "multiSelect":
      return Array.isArray(value) && value.length > 0 && value.every(isListValue)
  }
}

/** Whether `value` has the wire shape of some parameter's value. */
export function isParamValue(value: unknown): value is QueryParamValue {
  return TYPES.some((type) => valueFitsType(type, value))
}

function parseOption(value: unknown): QueryParamOption | null {
  if (typeof value !== "object" || value === null) return null
  const option = value as Record<string, unknown>
  if (!isListValue(option.value) || typeof option.label !== "string") return null
  return { value: option.value, label: option.label }
}

/**
 * A definition as it is stored, or `null` when it cannot be used: a name
 * that is not text or is empty, a type the server does not know, or an
 * option that is not a value with a label. A default that does not fit the
 * type is left out and the definition stays. A field that is not one of
 * ours is left out rather than guessed at.
 */
function parseDefinition(value: unknown): QueryParamDef | null {
  if (typeof value !== "object" || value === null) return null
  const stored = value as Record<string, unknown>
  if (typeof stored.name !== "string" || !stored.name) return null
  if (!isQueryParamType(stored.type)) return null
  const def: QueryParamDef = { name: stored.name, type: stored.type }
  if (stored.default !== undefined && valueFitsType(stored.type, stored.default)) {
    def.default = stored.default
  }
  if (stored.options !== undefined) {
    if (!Array.isArray(stored.options)) return null
    const options: QueryParamOption[] = []
    for (const entry of stored.options) {
      const option = parseOption(entry)
      // One bad option drops the definition: a dropdown that quietly lost a
      // choice would look valid and offer fewer values than were saved.
      if (!option) return null
      options.push(option)
    }
    if (options.length > 0) def.options = options
  }
  return def
}

/**
 * The definitions in `value`, or an empty list when it is not a list. A
 * malformed entry is dropped and its neighbours stay; of two entries with
 * one name the first stays, because the server refuses a name twice.
 */
export function parseDefinitions(value: unknown): QueryParamDef[] {
  if (!Array.isArray(value)) return []
  const kept: QueryParamDef[] = []
  const names = new Set<string>()
  for (const entry of value) {
    const def = parseDefinition(entry)
    if (!def || names.has(def.name)) continue
    names.add(def.name)
    kept.push(def)
  }
  return kept
}

/**
 * The values in `value` (name to value), or none when it is not an object.
 * A value must have the shape of its definition's type; a value whose
 * definition is gone (the name left the SQL) only needs the shape of some
 * value. A malformed value is dropped and its neighbours stay.
 */
export function parseValues(
  value: unknown,
  definitions: readonly QueryParamDef[]
): Record<string, QueryParamValue> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return {}
  const types = new Map(definitions.map((def) => [def.name, def.type]))
  const kept: [string, QueryParamValue][] = []
  for (const [name, entry] of Object.entries(value)) {
    const type = types.get(name)
    if (type ? valueFitsType(type, entry) : isParamValue(entry)) kept.push([name, entry])
  }
  // `fromEntries` defines each key as data, so a stored `__proto__` stays a
  // name and never reaches the object's prototype.
  return Object.fromEntries(kept)
}
