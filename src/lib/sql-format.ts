/**
 * A deliberately small SQL formatter.
 *
 * It does two things: puts the major clauses on their own lines and
 * uppercases the keywords it recognises. It does not re-indent
 * subqueries, align columns or rewrite expressions — a formatter that
 * rearranges SQL it does not fully parse is a formatter that eventually
 * mangles someone's query.
 *
 * Quoted strings, quoted identifiers and comments are copied through
 * untouched, so nothing inside them is ever treated as a keyword.
 */

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

const WORD = /[A-Za-z0-9_]/

type Token = { text: string; word: boolean }

/** Split into words, quoted/comment runs (kept verbatim), and everything else. */
function tokenize(sql: string): Token[] {
  const tokens: Token[] = []
  let i = 0
  while (i < sql.length) {
    const char = sql[i]
    const two = sql.slice(i, i + 2)

    if (two === "--" || char === "#") {
      const end = sql.indexOf("\n", i)
      const stop = end === -1 ? sql.length : end
      tokens.push({ text: sql.slice(i, stop), word: false })
      i = stop
      continue
    }
    if (two === "/*") {
      const end = sql.indexOf("*/", i + 2)
      const stop = end === -1 ? sql.length : end + 2
      tokens.push({ text: sql.slice(i, stop), word: false })
      i = stop
      continue
    }
    if (char === "'" || char === '"' || char === "`") {
      let j = i + 1
      while (j < sql.length) {
        if (sql[j] === "\\") {
          j += 2
          continue
        }
        if (sql[j] === char) {
          if (sql[j + 1] === char) {
            j += 2
            continue
          }
          j += 1
          break
        }
        j += 1
      }
      tokens.push({ text: sql.slice(i, j), word: false })
      i = j
      continue
    }
    if (WORD.test(char)) {
      let j = i
      while (j < sql.length && WORD.test(sql[j])) j += 1
      tokens.push({ text: sql.slice(i, j), word: true })
      i = j
      continue
    }
    if (/\s/.test(char)) {
      i += 1
      continue
    }
    tokens.push({ text: char, word: false })
    i += 1
  }
  return tokens
}

/** The clause starting at `index`, as its canonical upper-case text. */
function clauseAt(tokens: Token[], index: number): string | null {
  for (const clause of BREAK_BEFORE) {
    const parts = clause.split(" ")
    const matches = parts.every((part, k) => {
      const token = tokens[index + k]
      return token?.word && token.text.toUpperCase() === part
    })
    if (matches) return clause
  }
  return null
}

/**
 * `sql`, reformatted. Returns the input unchanged when there is nothing
 * to format, so the button is a no-op rather than a surprise on an empty
 * editor.
 */
export function formatSql(sql: string): string {
  if (!sql.trim()) return sql
  const tokens = tokenize(sql)
  let out = ""
  let i = 0

  // Spacing rules, kept to the few that make SQL readable: no space
  // before punctuation that closes or separates, none after an opening
  // bracket, and none between a function name and its bracket.
  const needsSpace = (next: string) =>
    out.length > 0 &&
    !out.endsWith("\n") &&
    !out.endsWith("(") &&
    !out.endsWith(".") &&
    !",)".includes(next[0]) &&
    !next.startsWith(".") &&
    !(next === "(" && /[A-Za-z0-9_]$/.test(out))

  while (i < tokens.length) {
    const clause = clauseAt(tokens, i)
    if (clause) {
      if (out.trim()) out = `${out.trimEnd()}\n`
      out += clause
      i += clause.split(" ").length
      continue
    }
    const token = tokens[i]
    let text = token.text
    if (token.word && INLINE_KEYWORDS.includes(text.toUpperCase())) {
      text = text.toUpperCase()
    }
    if (needsSpace(text)) out += " "
    out += text
    i += 1
  }
  return out.trim()
}
