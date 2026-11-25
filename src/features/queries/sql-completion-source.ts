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

/**
 * How far a column is lifted above the keywords. Used only in a "names"
 * place (QS round review R1, D8): in the general list names have no boost and
 * a keyword that matches what was typed ranks by CodeMirror's own score.
 */
const COLUMN_BOOST = 100

/**
 * How far a table is lifted above the keywords, and below the columns, after
 * `FROM`/`JOIN` (D8). In the general list a table gets no boost at all.
 */
const TABLE_BOOST = 99

/** One suggestion of ours, as CodeMirror wants it. */
function toCompletion(option: CompletionOption, place: "names" | "general"): Completion {
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
    // QS round review R1 (D8): only a "names" place boosts a table or column.
    // The message branch above and the `{{` parameter list are unchanged.
    boost:
      place === "general"
        ? 0
        : option.kind === "table"
          ? TABLE_BOOST
          : COLUMN_BOOST,
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
      options: answer.options.map((option) => toCompletion(option, answer.place)),
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
  return autocompletion({
    override: [source, keywords],
    // UX-9 browser review, SHOULD-FIX 3: CodeMirror's icons are emoji
    // glyphs (`🔑︎` for a keyword) that the console's fonts do not have, so
    // they drew as empty boxes. The icons are off (`icons` combines with
    // AND, so this wins over the editor's basic setup) and each option says
    // what it is in a small word at its end instead.
    icons: false,
    addToOptions: [{ render: renderKindLabel, position: 90 }],
  })
}

/** The word an option's `type` (set in `toCompletion`, or the keywords' own) is shown as. */
const KIND_LABEL: Record<string, string> = {
  property: "column",
  class: "table",
  variable: "parameter",
  keyword: "keyword",
  type: "type",
}

/** The small, muted kind word at the end of a completion option; nothing for a message. */
export function renderKindLabel(completion: { type?: string }): Node | null {
  const word = completion.type ? KIND_LABEL[completion.type.split(/\s+/)[0] ?? ""] : undefined
  if (!word) return null
  const label = document.createElement("span")
  label.className = "cm-completionKind"
  label.textContent = word
  label.setAttribute("aria-hidden", "true")
  return label
}
