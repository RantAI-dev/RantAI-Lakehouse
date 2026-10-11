import type { JoinSuggestions } from "@/services/contracts/dashboards"

/**
 * Joins in the chart builder (`BI-7`): the chart's mart plus up to four more
 * tables of the serving layer, read as one flat table. The shapes mirror
 * `lakehouse_bi::joins` (`type` on the wire; the server checks every rule at
 * save), so this module names them, builds the payload and labels what comes
 * back. Pure, so a test can pin each conversion without rendering the dialog.
 */

export type JoinType = "inner" | "left" | "right" | "full"
export type JoinOp = "=" | "<" | "<=" | ">" | ">="

/** One pair of a join condition as stored. */
export type JoinPairDef = { left: string; op: string; right: string }
/** One joined table as stored (`JoinSpec`). */
export type JoinDef = { mart: string; type: JoinType | string; on: JoinPairDef[] }

/** The builder's draft of a join: the same shape, with fields that may still be empty. */
export type JoinDraft = { mart: string; type: JoinType; on: JoinPairDef[] }

/** Most tables joined to the chart's own, and most pairs in one join. */
export const MAX_JOINS = 4
export const MAX_PAIRS = 4

export const JOIN_TYPES: { value: JoinType; label: string }[] = [
  { value: "left", label: "Left — every row of the table before" },
  { value: "inner", label: "Inner — only the matches" },
  { value: "right", label: "Right — every row of this table" },
  { value: "full", label: "Full — every row of both" },
]

export const JOIN_OPS: { value: JoinOp; label: string }[] = [
  { value: "=", label: "equals" },
  { value: "<", label: "is less than" },
  { value: "<=", label: "is at most" },
  { value: ">", label: "is greater than" },
  { value: ">=", label: "is at least" },
]

export function emptyJoin(mart = ""): JoinDraft {
  return { mart, type: "left", on: [{ left: "", op: "=", right: "" }] }
}

/** A saved definition's joins as drafts. */
export function joinDraftsFromDef(joins: readonly JoinDef[] | undefined): JoinDraft[] {
  return (joins ?? []).map((j) => ({
    mart: j.mart.replace(/^serving\./, ""),
    type: (["inner", "left", "right", "full"].includes(j.type) ? j.type : "left") as JoinType,
    on: j.on.map((p) => ({ ...p })),
  }))
}

function pairComplete(p: JoinPairDef): boolean {
  return !!p.left && !!p.right && !!p.op
}

/** A join that can be sent: a table, and every pair filled in with at least one equals. */
export function joinComplete(j: JoinDraft): boolean {
  return !!j.mart && j.on.length > 0 && j.on.every(pairComplete) && j.on.some((p) => p.op === "=")
}

/** What is still needed before the joins can be sent, in the builder's own words. */
export function joinProblems(joins: readonly JoinDraft[]): string[] {
  const out: string[] = []
  joins.forEach((j, i) => {
    const n = i + 1
    if (!j.mart) out.push(`Join ${n} table`)
    else if (!j.on.every(pairComplete)) out.push(`Join ${n} columns`)
    else if (!j.on.some((p) => p.op === "=")) out.push(`Join ${n} needs one "equals"`)
  })
  return out
}

/** The `joins` field to merge into the payload: none when there is none or one is not complete yet. */
export function joinsPayload(joins: readonly JoinDraft[]): { joins?: JoinDef[] } {
  if (!joins.length || joins.some((j) => !joinComplete(j))) return {}
  return { joins: joins.map((j) => ({ mart: j.mart, type: j.type, on: j.on.map((p) => ({ left: p.left, op: p.op, right: p.right })) })) }
}

/** The flat name of a joined column: `<alias>__<column>`. */
export function flatName(alias: string, column: string): string {
  return `${alias}__${column}`
}

/**
 * A flat column name as a person reads it: `table.column` for a joined
 * column, the name itself for any other (a result column that starts with two
 * underscores is the server's own and is left alone).
 */
export function displayColumn(name: string): string {
  if (name.startsWith("__")) return name
  const at = name.indexOf("__")
  return at > 0 && at + 2 < name.length ? `${name.slice(0, at)}.${name.slice(at + 2)}` : name
}

/**
 * `displayColumn` applied inside a longer label ("avg of mart_a__rooms" reads
 * "avg of mart_a.rooms"): each word that is a joined column is replaced.
 */
export function displayLabel(label: string): string {
  return label.replace(/[A-Za-z_][A-Za-z0-9_]*/g, (word) => displayColumn(word))
}

/**
 * The alias the server gives each joined table (`lakehouse_bi::joins::pick_alias`):
 * the table's name, with a number appended while the name is already an alias
 * or one of its flat column names would collide with a column already there.
 * `columnsOf` gives a table's columns; a table whose columns are not known yet
 * keeps its plain name.
 */
export function joinAliases(
  baseColumns: readonly string[],
  joins: readonly JoinDraft[],
  columnsOf: (mart: string) => readonly string[] | undefined,
): string[] {
  const taken = new Set<string>(baseColumns)
  const aliases = new Set<string>()
  const out: string[] = []
  for (const j of joins) {
    const cols = columnsOf(j.mart) ?? []
    let alias = j.mart
    for (let attempt = 1; attempt <= MAX_JOINS + 2; attempt++) {
      const candidate = attempt === 1 ? j.mart : `${j.mart}${attempt}`
      const clash = aliases.has(candidate) || cols.some((c) => taken.has(flatName(candidate, c)))
      if (!clash) { alias = candidate; break }
    }
    aliases.add(alias)
    for (const c of cols) taken.add(flatName(alias, c))
    out.push(alias)
  }
  return out
}

/** The columns a join's left side may name: the chart's own and those of the joins before it. */
export function leftChoices(
  baseColumns: readonly string[],
  joins: readonly JoinDraft[],
  index: number,
  columnsOf: (mart: string) => readonly string[] | undefined,
): string[] {
  const aliases = joinAliases(baseColumns, joins, columnsOf)
  const out = [...baseColumns]
  for (let i = 0; i < index; i++) {
    for (const c of columnsOf(joins[i].mart) ?? []) out.push(flatName(aliases[i], c))
  }
  return out
}

/**
 * Whether the equality pairs of a join cover the joined table's sorting key.
 * When they do not (or the table has no key), a row may match several rows of
 * the joined table and sums over its columns multiply; the builder warns,
 * it does not block (`lakehouse_bi::joins::covers_key`).
 */
export function coversKey(rightKey: readonly string[], pairs: readonly JoinPairDef[]): boolean {
  if (!rightKey.length) return false
  const equal = new Set(pairs.filter((p) => p.op === "=" && p.right).map((p) => p.right))
  return rightKey.every((k) => equal.has(k))
}

/** The pairs to apply by default: the suggestions marked `suggested`. */
export function pairsFromSuggestions(s: JoinSuggestions): JoinPairDef[] {
  const pairs = s.pairs.filter((p) => p.suggested).map((p) => ({ left: p.left, op: "=", right: p.right }))
  return pairs.length ? pairs : [{ left: "", op: "=", right: "" }]
}

/** Why rows may multiply, or null when the join's pairs cover the key. */
export function multiplyWarning(rightKey: readonly string[], j: JoinDraft): string | null {
  if (!j.mart || !j.on.every(pairComplete)) return null
  if (coversKey(rightKey, j.on)) return null
  return "These columns do not identify a row of the joined table, so rows may multiply and sums over it may add up more than once. Use max or average for its measures."
}
