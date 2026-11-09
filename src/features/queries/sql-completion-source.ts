"use client"

import { autocompletion } from "@codemirror/autocomplete"
import { StandardSQL, keywordCompletionSource } from "@codemirror/lang-sql"
import type { Extension } from "@codemirror/state"
import {
  inPlaceholder,
  sqlCompletionOptions,
  type ColumnState,
  type CompletionOption,
  type CompletionTable,
} from "@/lib/sql-completion"

/**
 * The console's own suggestions for the SQL editor.
 *
 * The SQL language offers the keywords. This adds what only the catalog
 * knows: the tables, and the columns of the table behind an alias; and,
 * after `{{`, the names of the tab's parameters. After a `{{` the keywords
 * stay out: a placeholder holds a parameter name and nothing else.
 *
 * The source is synchronous over the column cache. A keystroke never waits
 * for the network: the cache says whether the columns are there yet, and
 * the list says so when they are not.
 */

/**
 * The completion source type, taken from the language package that hands
 * the source out: the autocomplete package does not export it.
 */
type CompletionSource = ReturnType<typeof keywordCompletionSource>

/** What CodeMirror hands a source when it asks for suggestions. */
export type CompletionContext = Parameters<CompletionSource>[0]

/** What a source answers, without the promise and the empty answer. */
type CompletionResult = NonNullable<
  ReturnType<CompletionSource> extends infer R
    ? R extends Promise<infer U>
      ? U
      : R
    : never
>

/** One suggestion, as CodeMirror wants it. */
type Completion = CompletionResult["options"][number]

/** How far a column is lifted above the keywords the language extension offers. */
const COLUMN_BOOST = 100

/** How far a table is lifted above the keywords, and below the columns. */
const TABLE_BOOST = 99

/** One suggestion of ours, as CodeMirror wants it. */
function toCompletion(option: CompletionOption): Completion {
  if (option.kind === "message") {
    // A message says something about the catalog; it is not part of the
    // query and inserts nothing. An empty text would be read as "no
    // apply", and CodeMirror would insert the label instead.
    return {
      label: option.label,
      type: "text",
      detail: option.detail,
      apply: () => {},
      boost: COLUMN_BOOST,
    }
  }
  return {
    label: option.label,
    type: option.kind === "column" ? "property" : option.kind === "parameter" ? "variable" : "class",
    detail: option.detail,
    apply: option.apply,
    boost: option.kind === "table" ? TABLE_BOOST : COLUMN_BOOST,
  }
}

/**
 * A completion source that answers from the tables and the column cache.
 *
 * The cache is read through a function, not through a value: the source is
 * built once, and the cache it reads changes while the person types.
 */
export function createSqlCompletionSource({
  tables,
  columns,
  parameters,
}: {
  /** The tables the catalog shows. */
  tables: CompletionTable[]
  /** The columns fetched so far, keyed by the catalog id of the table. */
  columns: () => Record<string, ColumnState>
  /** The names of the open tab's parameter definitions, read when `{{` is completed. */
  parameters: () => readonly string[]
}): CompletionSource {
  return (context: CompletionContext): CompletionResult => {
    const sql = context.state.doc.toString()
    const answer = sqlCompletionOptions({
      sql,
      cursor: context.pos,
      tables,
      columns: columns(),
      parameters: parameters(),
    })
    return {
      from: answer.from,
      options: answer.options.map(toCompletion),
    }
  }
}

/**
 * The keywords the SQL language offers.
 *
 * The editor asks for the standard dialect's keywords directly, so they come
 * out as the dialect spells them, not upper-cased. Naming the answering
 * sources replaces the language's own completion, which the `sql()` extension
 * would install.
 */
const keywordSource = keywordCompletionSource(StandardSQL, false)

/**
 * The keywords, without the ones that would show after a `{{`.
 *
 * A placeholder holds a parameter name, so a keyword cannot be written
 * there and does not belong in the list (#422 S2).
 */
const keywords: CompletionSource = (context) => {
  if (inPlaceholder(context.state.doc.toString(), context.pos)) return null
  return keywordSource(context)
}

/**
 * The extension that gives the SQL editor its suggestions.
 *
 * It names the sources that answer, so the console's own source and the
 * language's keywords are the only two: the keywords stay out of a
 * placeholder, and the console's source does not offer a parameter name
 * inside a comment or a quoted run.
 */
export function sqlCompletionExtension(source: CompletionSource): Extension {
  return autocompletion({ override: [source, keywords] })
}
