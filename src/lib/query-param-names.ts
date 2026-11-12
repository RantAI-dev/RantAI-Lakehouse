import type {
  QueryParamDef,
  QueryParamInputs,
  QueryParamType,
  QueryParamValue,
} from "@/services/contracts/queries"
import { valueFitsType } from "@/lib/query-params"
import { tokenizeSql } from "@/lib/sql-text"

/**
 * Finding the `{{name}}` placeholders in a tab's SQL (`QS-5`), and keeping
 * the tab's definitions in step with them.
 *
 * The console reads the SQL with its own tokenizer, so a placeholder in a
 * comment or in a quoted string makes no field. The server stays the judge:
 * it refuses a placeholder inside quotes and a `{{` that is not a
 * placeholder. The shape of a placeholder follows the server's scanner
 * (`query_params.rs`, `parse_placeholder`): blanks, a name, optionally
 * `.start` or `.end`, blanks, `}}`.
 */

/** A name: 1 to 64 ASCII letters, digits or `_`, not starting with a digit (`BI-11`'s rule). */
const MAX_NAME_LEN = 64

/** `{{ name }}`, `{{ name.start }}` or `{{ name.end }}`; `\s` is the server's `is_whitespace`. */
const PLACEHOLDER = /\{\{\s*([A-Za-z_][A-Za-z0-9_]*)(?:\.(start|end))?\s*\}\}/g

/** A name the SQL uses, and the ways it uses it. */
export type ParamMention = {
  name: string
  /** Written as `{{name}}` at least once. */
  plain: boolean
  /** Written as `{{name.start}}` or `{{name.end}}` at least once. */
  ranged: boolean
}

/**
 * The SQL with every comment and quoted run blanked out. The filler is a
 * character no placeholder can contain, so what is left around a blanked
 * run never joins into a placeholder.
 */
function codeOnly(sql: string): string {
  let out = ""
  let cursor = 0
  for (const token of tokenizeSql(sql)) {
    if (token.kind !== "comment" && token.kind !== "quoted") continue
    out += sql.slice(cursor, token.from) + "\0".repeat(token.to - token.from)
    cursor = token.to
  }
  return out + sql.slice(cursor)
}

/** The names `sql` uses, in the order each first appears, with how each is written. */
export function findParamNames(sql: string): ParamMention[] {
  const found = new Map<string, ParamMention>()
  for (const match of codeOnly(sql).matchAll(PLACEHOLDER)) {
    const name = match[1]
    if (name === undefined || name.length > MAX_NAME_LEN) continue
    const mention = found.get(name) ?? { name, plain: false, ranged: false }
    if (match[2] === undefined) mention.plain = true
    else mention.ranged = true
    found.set(name, mention)
  }
  return [...found.values()]
}

/**
 * The definition a name gets when nobody has defined it: a date range when
 * the SQL writes it only as `.start` / `.end`, text otherwise. A name
 * written both ways is text; the server refuses it and says why.
 */
function implicitDefinition({ name, plain, ranged }: ParamMention): QueryParamDef {
  return { name, type: ranged && !plain ? "dateRange" : "text" }
}

/**
 * The definitions of the names `sql` uses, in the order they first appear.
 * A name with a stored definition gets it; a name without one gets the
 * implicit definition. A stored definition whose name `sql` does not use is
 * left out: it stays in the tab, not in the fields and not in a request.
 */
export function activeParameters(
  sql: string,
  definitions: readonly QueryParamDef[]
): QueryParamDef[] {
  return findParamNames(sql).map(
    (mention) => definitions.find((def) => def.name === mention.name) ?? implicitDefinition(mention)
  )
}

/** Whether `def` is what {@link implicitDefinition} would have made: nobody edited it. */
function isUntouched(def: QueryParamDef): boolean {
  return (def.type === "text" || def.type === "dateRange") && def.default === undefined && def.options === undefined
}

/**
 * The tab's definitions after its SQL changed: a name the SQL gained gets
 * the implicit definition, appended. A name the SQL lost keeps its
 * definition, so a cut and paste does not lose it, unless nobody edited it
 * and no value was typed: that definition is what the name would get again,
 * and the editor closes `{{` by itself, so typing `{{region}}` passes
 * through `{{r}}`, `{{re}}` and more. Returns `definitions` itself when
 * nothing changes.
 */
export function reconcileDefinitions(
  sql: string,
  definitions: readonly QueryParamDef[],
  values: Readonly<Record<string, QueryParamValue>>
): QueryParamDef[] {
  const used = findParamNames(sql)
  const usedNames = new Set(used.map((mention) => mention.name))
  const kept = definitions.filter(
    (def) => usedNames.has(def.name) || !isUntouched(def) || Object.hasOwn(values, def.name)
  )
  const known = new Set(kept.map((def) => def.name))
  const added = used.filter((mention) => !known.has(mention.name)).map(implicitDefinition)
  if (added.length === 0 && kept.length === definitions.length) return definitions as QueryParamDef[]
  return [...kept, ...added]
}

/**
 * The tab's definitions and values after one definition changed. The
 * definition replaces the one of the same name, or is appended. A value
 * that no longer has the wire shape of the new type goes: a text left in a
 * number field would be sent as the wrong thing, and the server's refusal
 * would blame a value the person did not just type.
 */
export function replaceDefinition(
  definitions: readonly QueryParamDef[],
  values: Readonly<Record<string, QueryParamValue>>,
  def: QueryParamDef
): { parameters: QueryParamDef[]; values: Record<string, QueryParamValue> } {
  const exists = definitions.some((stored) => stored.name === def.name)
  const parameters = exists
    ? definitions.map((stored) => (stored.name === def.name ? def : stored))
    : [...definitions, def]
  const kept = { ...values }
  if (Object.hasOwn(kept, def.name) && !valueFitsType(def.type, kept[def.name])) {
    delete kept[def.name]
  }
  return { parameters, values: kept }
}

/**
 * `def` with another type. The default stays only when it fits the new type,
 * and the options stay only for a dropdown or a multi-select.
 */
export function retypeDefinition(def: QueryParamDef, type: QueryParamType): QueryParamDef {
  const next: QueryParamDef = { name: def.name, type }
  if (def.default !== undefined && valueFitsType(type, def.default)) next.default = def.default
  if ((type === "dropdown" || type === "multiSelect") && def.options) next.options = def.options
  return next
}

/**
 * What a run, an estimate or a save sends for `definitions`, the
 * definitions of the names the SQL uses right now (`activeParameters`).
 *
 * With none, nothing: the request is the one sent before `QS-5`. With some,
 * the definitions go whole (the server needs them to check the types, and
 * the default lives in the definition) and the values go for the names that
 * have one. A value whose name is not in `definitions` is left out, because
 * a name that left the SQL has no business in the request.
 */
export function paramInputs(
  definitions: readonly QueryParamDef[],
  values: Readonly<Record<string, QueryParamValue>>
): QueryParamInputs {
  if (definitions.length === 0) return {}
  const given: Record<string, QueryParamValue> = {}
  for (const def of definitions) {
    if (Object.hasOwn(values, def.name)) given[def.name] = values[def.name]
  }
  return { parameters: given, definitions: [...definitions] }
}

/**
 * Whether some name in `definitions` has nothing to run with: no value that
 * fits its type and no default. The server refuses such a run with "Fill in
 * {{name}} to run this query." An estimate is skipped instead of refused, so
 * the panel does not show an error for a field the person has not reached.
 */
export function missingRequiredValue(
  definitions: readonly QueryParamDef[],
  values: Readonly<Record<string, QueryParamValue>>
): boolean {
  return firstMissingValueName(definitions, values) !== null
}

/**
 * The first name in `definitions` (parameter order) that has nothing to run
 * with, or `null` when every name has a value. The estimate panel quotes it
 * as `{{name}}`, the spelling of the server's run sentence (QS-5-AC14, plan
 * 73 D6).
 */
export function firstMissingValueName(
  definitions: readonly QueryParamDef[],
  values: Readonly<Record<string, QueryParamValue>>
): string | null {
  const missing = definitions.find((def) => {
    const value = Object.hasOwn(values, def.name) ? values[def.name] : def.default
    return value === undefined || !valueFitsType(def.type, value)
  })
  return missing?.name ?? null
}

/**
 * The names in `names` (the ones the SQL uses) that `sentence`, the server's refusal, quotes:
 * as `{{name}}`, `{{name.start}}`, `{{name.end}}` or `parameter 'name'`.
 * The wire gives a sentence and no field, so this is how the field the
 * server blames is found. A name is matched whole: `a` is not found in
 * `{{ab}}`. A sentence about the query as a whole names no field.
 */
export function fieldsNamedBy(sentence: string, names: readonly string[]): Set<string> {
  const named = new Set<string>()
  for (const name of names) {
    // A name is letters, digits and `_` (`findParamNames`), so it needs no escaping.
    const quoted = new RegExp(`\\{\\{\\s*${name}(?:\\.(?:start|end))?\\s*\\}\\}|parameter '${name}'`)
    if (quoted.test(sentence)) named.add(name)
  }
  return named
}
