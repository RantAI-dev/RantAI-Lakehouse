/**
 * A deliberately small SQL formatter.
 *
 * It does three things: puts the major clauses on their own lines, writes
 * the keywords it recognises the way the reader asked for, and indents the
 * body of a clause when asked to. It does not align columns or rewrite
 * expressions — a formatter that rearranges SQL it does not fully parse
 * is a formatter that eventually mangles someone's query.
 *
 * Quoted strings, quoted identifiers and comments are copied through
 * untouched, so nothing inside them is ever treated as a keyword.
 *
 * The indent setting puts the body of a clause on the line after its
 * keyword. A clause inside brackets sits one level deeper than the line
 * that holds the opening bracket, but the closing bracket stays attached
 * to the last token of the body: this formatter reads the clauses it
 * knows and indents them, it does not re-indent a subquery's own
 * expression.
 */

import { tokenizeSql, type SqlToken } from "./sql-text"

/** How the keywords the formatter recognises are written. */
export type KeywordCase = "upper" | "lower" | "preserve"

/** What the formatter may be told to do. */
export type FormatOptions = {
  /** How the keywords are written. Upper case when not given. */
  keywordCase?: KeywordCase
  /** How far the body of a clause is indented, in spaces. None when not given. */
  indent?: 0 | 2 | 4
}

/** Clauses that start a new line. Longest first, so "GROUP BY" wins over "GROUP". */
const BREAK_BEFORE = [
  "LEFT OUTER JOIN",
  "RIGHT OUTER JOIN",
  "FULL OUTER JOIN",
  "CROSS JOIN",
  "INNER JOIN",
  "LEFT JOIN",
  "RIGHT JOIN",
  "GROUP BY",
  "ORDER BY",
  "UNION ALL",
  "PARTITION BY",
  "SELECT",
  "FROM",
  "WHERE",
  "HAVING",
  "LIMIT",
  "OFFSET",
  "UNION",
  "JOIN",
  "SETTINGS",
]

/** Words that read as keywords but stay on the current line. */
const INLINE_KEYWORDS = [
  "AND",
  "OR",
  "NOT",
  "ON",
  "AS",
  "IN",
  "IS",
  "NULL",
  "LIKE",
  "BETWEEN",
  "CASE",
  "WHEN",
  "THEN",
  "ELSE",
  "END",
  "DESC",
  "ASC",
  "WITH",
  "DISTINCT",
  "USING",
]

/** A clause the formatter recognises at some place in the buffer. */
type Clause = {
  /** The clause's canonical upper-case text, e.g. `GROUP BY`. */
  canonical: string
  /** How many words the clause is made of. */
  words: number
  /** The clause as it was typed, whitespace and all. */
  typed: string
}

/** The clause starting at `index`, if there is one. */
function clauseAt(tokens: SqlToken[], index: number, sql: string): Clause | null {
  for (const clause of BREAK_BEFORE) {
    const parts = clause.split(" ")
    const matches = parts.every((part, k) => {
      const token = tokens[index + k]
      return token?.kind === "word" && token.text.toUpperCase() === part
    })
    if (!matches) continue
    const first = tokens[index]
    const last = tokens[index + parts.length - 1]
    return {
      canonical: clause,
      words: parts.length,
      typed: sql.slice(first.from, last.to),
    }
  }
  return null
}

/** The clause as the reader asked for it. */
function clauseText(clause: Clause, keywordCase: KeywordCase): string {
  if (keywordCase === "lower") return clause.canonical.toLowerCase()
  if (keywordCase === "preserve") return clause.typed
  return clause.canonical
}

/** A keyword as the reader asked for it. */
function keywordText(text: string, keywordCase: KeywordCase): string {
  if (keywordCase === "lower") return text.toLowerCase()
  if (keywordCase === "preserve") return text
  return text.toUpperCase()
}

/**
 * `sql`, reformatted. Returns the input unchanged when there is nothing
 * to format, so the button is a no-op rather than a surprise on an empty
 * editor.
 *
 * Without options the result is exactly what this formatter has always
 * produced: upper-case keywords and one clause per line.
 */
export function formatSql(sql: string, options?: FormatOptions): string {
  const keywordCase = options?.keywordCase ?? "upper"
  const indent = options?.indent ?? 0
  if (!sql.trim()) return sql

  const tokens = tokenizeSql(sql)
  const lines: string[] = []
  // The line being written, without its indent, and the level it is at.
  let line = ""
  let level = 0
  // The level the body of the current clause is written at: one deeper
  // than the clause's own line.
  let bodyLevel = 0
  // The body of a clause goes on the line after its keyword.
  let bodyPending = false
  // Every open bracket, with the level its content is written at.
  const brackets: { line: number; level: number }[] = []

  /** Finish the current line and start a new one at `at`. */
  const startLine = (at: number) => {
    if (line !== "") {
      lines.push(indent > 0 ? " ".repeat(level * indent) + line : line)
      line = ""
    }
    level = at
  }

  // Spacing rules, kept to the few that make SQL readable: no space
  // before punctuation that closes or separates, none after an opening
  // bracket, and none between a function name and its bracket.
  const needsSpace = (next: string) =>
    line !== "" &&
    !line.endsWith("(") &&
    !line.endsWith(".") &&
    !",)".includes(next[0]) &&
    !next.startsWith(".") &&
    !(next === "(" && /[A-Za-z0-9_]$/.test(line))

  let i = 0
  while (i < tokens.length) {
    const clause = clauseAt(tokens, i, sql)
    if (clause) {
      // A clause inside brackets sits one level deeper than the line that
      // holds the opening bracket.
      const at = brackets.length > 0 ? brackets[brackets.length - 1].level : 0
      startLine(at)
      line = clauseText(clause, keywordCase)
      bodyLevel = at + 1
      bodyPending = indent > 0
      i += clause.words
      continue
    }
    const token = tokens[i]
    let text = token.text
    if (token.kind === "word" && INLINE_KEYWORDS.includes(text.toUpperCase())) {
      text = keywordText(text, keywordCase)
    }
    if (bodyPending) {
      startLine(bodyLevel)
      bodyPending = false
    } else if (line === "") {
      // The first token of the buffer, before any clause.
      startLine(0)
    }
    if (needsSpace(text)) line += " "
    line += text
    if (text === "(") {
      // The content of a bracket is one level deeper than the line the
      // bracket is on, and a bracket inside a bracket one level deeper
      // again.
      const sameLine = brackets.filter((bracket) => bracket.line === level).length
      brackets.push({ line: level, level: level + 1 + sameLine })
    } else if (text === ")") {
      brackets.pop()
    }
    i += 1
  }
  if (line !== "") {
    lines.push(indent > 0 ? " ".repeat(level * indent) + line : line)
  }
  return lines.join("\n")
}
