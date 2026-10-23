"use client"

import { StandardSQL, type keywordCompletionSource } from "@codemirror/lang-sql"
import type { Extension } from "@codemirror/state"
import {
  sqlCompletionOptions,
  type ColumnState,
  type CompletionOption,
  type CompletionTable,
} from "@/lib/sql-completion"

/**
 * The console's own suggestions for the SQL editor.
 *
 * The editor's language extension already offers the SQL keywords. This
 * adds what only the catalog knows: the tables, and the columns of the
 * table behind an alias.
 *
 * The source is synchronous over the column cache. A keystroke never waits
 * for the network: the cache says whether the columns are there yet, and
 * the list says so when they are not.
 */

/**
 * The completion source type, taken from the language package that is a
 * dependency: the autocomplete package is not one, so its types are read
 * through the source it hands out.
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
    type: option.kind === "column" ? "property" : "class",
    detail: option.detail,
    apply: option.apply,
    boost: option.kind === "column" ? COLUMN_BOOST : TABLE_BOOST,
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
}: {
  /** The tables the catalog shows. */
  tables: CompletionTable[]
  /** The columns fetched so far, keyed by the catalog id of the table. */
  columns: () => Record<string, ColumnState>
}): CompletionSource {
  return (context: CompletionContext): CompletionResult => {
    const sql = context.state.doc.toString()
    const answer = sqlCompletionOptions({
      sql,
      cursor: context.pos,
      tables,
      columns: columns(),
    })
    return {
      from: answer.from,
      options: answer.options.map(toCompletion),
    }
  }
}

/**
 * The extension that adds the console's suggestions to the SQL language.
 *
 * The language's data facet is additive: the keywords the SQL extension
 * offers stay, and this source is asked beside them.
 */
export function sqlCompletionExtension(source: CompletionSource): Extension {
  return StandardSQL.language.data.of({ autocomplete: source })
}
