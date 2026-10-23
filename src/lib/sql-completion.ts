/**
 * Turning SQL text and the catalog into the suggestions the editor offers.
 *
 * Pure on purpose: the editor asks for suggestions on every keystroke, and
 * a keystroke may not wait for the network. Everything this module needs —
 * the tables the catalog shows, the columns already fetched — arrives as an
 * argument, so the same code answers whether the columns are there yet or
 * not.
 *
 * The SQL is read through the console's one scanner (`./sql-text`), so a
 * quoted string that holds a dot is not mistaken for a qualified name, and
 * a comment is not mistaken for code.
 */

import type { DataLayer } from "./status"
import { tokenizeSql, type SqlToken } from "./sql-text"

/** A table the catalog shows, and the names a person may write for it. */
export type CompletionTable = {
  /** The catalog id of the table — the key of the column cache. */
  id: string
  /** The name a person reads, e.g. `Mart Wisman`. */
  label: string
  /** The SQL name to insert, e.g. `silver.mart_wisman`. Empty when the catalog does not say. */
  reference: string
  /** The layer the catalog files the table under; the schema browser groups by it, the completion ignores it. */
  layer: DataLayer
  /** Every name the table answers to: its id and its display name (`namespace.name`). */
  names: string[]
}

/** What the console knows about one table's columns right now. */
export type ColumnState =
  | { status: "loading" }
  | { status: "ready"; columns: { name: string; dataType: string }[] }
  /** `reason` is the catalog's own refusal, when it gave one; the completion note stays the same either way. */
  | { status: "failed"; reason?: string }

/** One suggestion. */
export type CompletionOption = {
  /** What the person reads in the list. */
  label: string
  /** What the suggestion is: a column, a table, or a note about the catalog. */
  kind: "column" | "table" | "message"
  /** The type shown next to the label, e.g. `Int64`. */
  detail?: string
  /**
   * The text to insert. Absent means "insert nothing" — a message says
   * something about the catalog, it does not belong in the query.
   */
  apply?: string
}

/** A table the statement names, and the name it was written with. */
export type NamedTable = {
  /** The catalog id of the table. */
  id: string
  /** The reference as it was written, e.g. `iceberg.bronze.demo_orders`. */
  reference: string
}

/** What the completion needs to know. */
export type CompletionRequest = {
  /** The whole buffer. */
  sql: string
  /** Where the cursor is, as an offset into `sql`. */
  cursor: number
  /** The tables the catalog shows. */
  tables: CompletionTable[]
  /** The columns fetched so far, keyed by the catalog id of the table. */
  columns: Record<string, ColumnState>
}

/** What the completion answers. */
export type CompletionAnswer = {
  /** Where the word being completed starts. */
  from: number
  /** The suggestions, in the order they should be read. */
  options: CompletionOption[]
}

/**
 * The words that end a `FROM` clause: after one of them a word is not a
 * table any more.
 */
const ENDS_FROM = new Set([
  "GROUP",
  "HAVING",
  "LIMIT",
  "OFFSET",
  "ON",
  "ORDER",
  "QUALIFY",
  "SELECT",
  "UNION",
  "USING",
  "VALUES",
  "WHERE",
  "WINDOW",
])

/**
 * The words that may follow a table reference without being its alias.
 *
 * The list is short on purpose: a word that is not in it is read as an
 * alias, and a table the catalog does not know has no columns to offer
 * anyway.
 */
const NOT_AN_ALIAS = new Set([
  "ALL",
  "AND",
  "ANY",
  "AS",
  "ASC",
  "BETWEEN",
  "BY",
  "CASE",
  "CROSS",
  "DELETE",
  "DESC",
  "DISTINCT",
  "ELSE",
  "END",
  "EXISTS",
  "EXPLAIN",
  "FALSE",
  "FETCH",
  "FOR",
  "FROM",
  "FULL",
  "GROUP",
  "HAVING",
  "IN",
  "INNER",
  "INSERT",
  "INTO",
  "IS",
  "JOIN",
  "LATERAL",
  "LEFT",
  "LIKE",
  "LIMIT",
  "NOT",
  "NULL",
  "OFFSET",
  "ON",
  "OR",
  "ORDER",
  "OUTER",
  "RIGHT",
  "SELECT",
  "SET",
  "THEN",
  "TRUE",
  "UNION",
  "UPDATE",
  "USING",
  "VALUES",
  "WHEN",
  "WHERE",
  "WITH",
])

/**
 * The dotted name right before the word being completed: `o` in `o.am`,
 * `silver.orders` in `silver.orders.amount`.
 */
const QUALIFIER = /((?:[A-Za-z_][A-Za-z0-9_]*\.)*[A-Za-z_][A-Za-z0-9_]*)\.$/

/** A table reference the statement makes. */
type Reference = {
  /** The table the reference names. */
  table: CompletionTable
  /** The reference as it was written. */
  written: string
  /** The alias the statement gives the table, when it gives one. */
  alias: string | null
}

/**
 * The table a written reference names.
 *
 * Three ways, in order: the table's own SQL name, one of the names the
 * catalog answers to, and the last part of the reference against the
 * catalog id — which is how `iceberg.bronze.demo_orders` finds the Bronze
 * asset whose id is `demo_orders`.
 *
 * SQL writes unquoted identifiers in any case, so the comparison ignores
 * it; what is inserted is always the catalog's own spelling.
 */
function resolveTable(written: string, tables: CompletionTable[]): CompletionTable | null {
  const wanted = written.toLowerCase()
  const exact = tables.find((table) => table.reference.toLowerCase() === wanted)
  if (exact) return exact
  const named = tables.find((table) =>
    table.names.some((name) => name.toLowerCase() === wanted)
  )
  if (named) return named
  const last = written.split(".").pop() ?? written
  return tables.find((table) => table.id.toLowerCase() === last.toLowerCase()) ?? null
}

/**
 * Where the chain of words joined by dots ends.
 *
 * The parts have to sit next to each other: `silver.orders` is one name,
 * while `o. FROM` is a column and the keyword that follows it.
 */
function chainEnd(tokens: SqlToken[], start: number): number {
  let last = start
  while (
    tokens[last + 1]?.kind === "punctuation" &&
    tokens[last + 1]?.text === "." &&
    tokens[last + 2]?.kind === "word" &&
    tokens[last + 1]?.from === tokens[last]?.to &&
    tokens[last + 2]?.from === tokens[last + 1]?.to
  ) {
    last += 2
  }
  return last
}

/**
 * The alias that follows a table reference, if it has one.
 *
 * Both `FROM t o` and `FROM t AS o` name the same table; a word that is a
 * keyword names no table at all.
 */
function aliasAfter(tokens: SqlToken[], last: number): string | null {
  let next = last + 1
  if (tokens[next]?.kind === "word" && tokens[next]?.text.toUpperCase() === "AS") {
    next += 1
  }
  const word = tokens[next]
  if (!word || word.kind !== "word") return null
  if (NOT_AN_ALIAS.has(word.text.toUpperCase())) return null
  return word.text
}

/**
 * The table references the statement makes.
 *
 * A reference is a word, or a chain of words joined by dots, that the
 * catalog knows. Only a reference in a table position (`FROM`, `JOIN`) can
 * have an alias: an alias names the table it follows.
 */
function references(tokens: SqlToken[], tables: CompletionTable[]): Reference[] {
  const found: Reference[] = []
  let inTableClause = false
  let i = 0
  while (i < tokens.length) {
    const token = tokens[i]
    if (token.kind !== "word") {
      i += 1
      continue
    }
    const word = token.text.toUpperCase()
    if (word === "FROM" || word === "JOIN") {
      inTableClause = true
      i += 1
      continue
    }
    if (ENDS_FROM.has(word)) {
      inTableClause = false
      i += 1
      continue
    }
    const last = chainEnd(tokens, i)
    let written = token.text
    for (let k = i + 2; k <= last; k += 2) written += `.${tokens[k]?.text ?? ""}`
    const table = resolveTable(written, tables)
    if (table) {
      found.push({
        table,
        written,
        alias: inTableClause ? aliasAfter(tokens, last) : null,
      })
    }
    i = last + 1
  }
  return found
}

/** Where the word the cursor is in starts. */
function wordStart(sql: string, cursor: number): number {
  let from = cursor
  while (from > 0 && /[A-Za-z0-9_]/.test(sql[from - 1] ?? "")) from -= 1
  return from
}

/** The dotted name right before the word being completed, if there is one. */
function qualifierBefore(sql: string, from: number): string | null {
  const match = QUALIFIER.exec(sql.slice(0, from))
  return match?.[1] ?? null
}

/** The last word before a place in the buffer, in upper case. */
function lastWordBefore(tokens: SqlToken[], position: number): string | null {
  let word: string | null = null
  for (const token of tokens) {
    if (token.kind !== "word") continue
    if (token.to > position) break
    word = token.text.toUpperCase()
  }
  return word
}

/**
 * The table a qualifier names.
 *
 * An alias the statement gives a table comes first — `FROM silver.orders o`
 * then `o.` means that table — and after that the qualifier as a table
 * name, written or not.
 */
function resolveQualifier(
  qualifier: string,
  tokens: SqlToken[],
  tables: CompletionTable[]
): CompletionTable | null {
  const wanted = qualifier.toLowerCase()
  for (const reference of references(tokens, tables)) {
    if (reference.alias?.toLowerCase() === wanted) return reference.table
  }
  return resolveTable(qualifier, tables)
}

/** The columns of one table, or the note that says they are not there yet. */
function columnOptions(state: ColumnState | undefined): CompletionOption[] {
  if (!state) return []
  if (state.status === "loading") {
    return [{ label: "Loading columns…", kind: "message" }]
  }
  if (state.status === "failed") {
    return [{ label: "Columns could not be loaded", kind: "message" }]
  }
  return state.columns.map((column) => ({
    label: column.name,
    kind: "column",
    detail: column.dataType,
    apply: column.name,
  }))
}

/** The tables that can be written in a query, by their SQL name. */
function tableOptions(tables: CompletionTable[]): CompletionOption[] {
  return tables
    .filter((table) => table.reference !== "")
    .map((table) => ({
      label: table.reference,
      kind: "table",
      detail: table.label,
      apply: table.reference,
    }))
}

/** The columns of the tables the statement names, in the order it names them. */
function namedColumnOptions(
  tokens: SqlToken[],
  tables: CompletionTable[],
  columns: Record<string, ColumnState>
): CompletionOption[] {
  const options: CompletionOption[] = []
  const seen = new Set<string>()
  for (const reference of references(tokens, tables)) {
    if (seen.has(reference.table.id)) continue
    seen.add(reference.table.id)
    const state = columns[reference.table.id]
    if (state?.status !== "ready") continue
    options.push(...columnOptions(state))
  }
  return options
}

/**
 * The tables the buffer names, each with the reference it was written with.
 *
 * The same table may be named twice; the caller decides what to do with
 * that, and the console fetches its columns once.
 */
export function tablesInStatement(sql: string, tables: CompletionTable[]): NamedTable[] {
  return references(tokenizeSql(sql), tables).map((reference) => ({
    id: reference.table.id,
    reference: reference.written,
  }))
}

/**
 * The suggestions for the place the cursor is in.
 *
 * Three places, in order:
 * - after a qualifier (`o.`, `silver.orders.`): the columns of that one
 *   table, or the note that says they are not there yet;
 * - after `FROM` or `JOIN`: the tables;
 * - anywhere else: the columns of the tables the statement names, then the
 *   tables.
 *
 * Keywords are not on this list. The SQL language extension already offers
 * them, and listing them here as well would show every keyword twice.
 */
export function sqlCompletionOptions({
  sql,
  cursor,
  tables,
  columns,
}: CompletionRequest): CompletionAnswer {
  const tokens = tokenizeSql(sql)
  const from = wordStart(sql, cursor)

  const qualifier = qualifierBefore(sql, from)
  if (qualifier) {
    const table = resolveQualifier(qualifier, tokens, tables)
    if (table) {
      // The table behind the qualifier is known: its columns are the only
      // thing that fits here. A table whose columns have not been fetched
      // yet says so, rather than offering another table's columns.
      return { from, options: columnOptions(columns[table.id]) }
    }
    // The qualifier names nothing the catalog knows. The tables the
    // statement names may still help, so the general case answers.
  }

  const previous = lastWordBefore(tokens, from)
  if (previous === "FROM" || previous === "JOIN") {
    return { from, options: tableOptions(tables) }
  }

  return {
    from,
    options: [...namedColumnOptions(tokens, tables, columns), ...tableOptions(tables)],
  }
}
