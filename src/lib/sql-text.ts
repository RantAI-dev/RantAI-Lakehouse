/**
 * Reading SQL text well enough to decide what the UI should do with it —
 * not to execute it. The server has its own guard (`routes/query.rs`);
 * this only stops the console from asking pointless questions about a
 * buffer that holds nothing but comments.
 *
 * One scanner for the console: the formatter (`./sql-format`) reads its
 * tokens here too, and the statement splitter is built on the same tokens.
 * A second scanner would be a second set of rules about where a quoted
 * string ends and where a comment starts, and the two would drift apart
 * exactly where it matters — inside a string that holds a semicolon.
 */

/** What a token is: a word, a quoted run, a comment, or one punctuation mark. */
export type SqlTokenKind = "word" | "quoted" | "comment" | "punctuation"

export type SqlToken = {
  /** The token's text, exactly as it appears in the input. */
  text: string
  /** Where the token starts in the input. */
  from: number
  /** Where the token ends in the input, exclusive. */
  to: number
  /** What the token is. */
  kind: SqlTokenKind
}

/** A word is a run of letters, digits and underscores; anything else is not. */
const WORD = /[A-Za-z0-9_]/

/**
 * Split SQL into words, quoted runs, comments and punctuation, each with
 * its offsets in the input. Whitespace is not a token.
 *
 * A quoted run and a comment are read whole, so a semicolon or a comment
 * marker inside one of them is never a separator and never a comment.
 */
export function tokenizeSql(sql: string): SqlToken[] {
  const tokens: SqlToken[] = []
  let i = 0
  while (i < sql.length) {
    const char = sql[i]
    const two = sql.slice(i, i + 2)

    if (two === "--" || char === "#") {
      const end = sql.indexOf("\n", i)
      const stop = end === -1 ? sql.length : end
      tokens.push({ text: sql.slice(i, stop), from: i, to: stop, kind: "comment" })
      i = stop
      continue
    }
    if (two === "/*") {
      const end = sql.indexOf("*/", i + 2)
      const stop = end === -1 ? sql.length : end + 2
      tokens.push({ text: sql.slice(i, stop), from: i, to: stop, kind: "comment" })
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
      tokens.push({ text: sql.slice(i, j), from: i, to: j, kind: "quoted" })
      i = j
      continue
    }
    if (WORD.test(char)) {
      let j = i
      while (j < sql.length && WORD.test(sql[j])) j += 1
      tokens.push({ text: sql.slice(i, j), from: i, to: j, kind: "word" })
      i = j
      continue
    }
    if (/\s/.test(char)) {
      i += 1
      continue
    }
    tokens.push({ text: char, from: i, to: i + 1, kind: "punctuation" })
    i += 1
  }
  return tokens
}

/**
 * `sql` with line comments (`--`, `#`) and block comments removed.
 *
 * Everything else — whitespace included — is kept character for character,
 * so the result still lines up with the original text.
 */
export function stripSqlComments(sql: string): string {
  let out = ""
  let cursor = 0
  for (const token of tokenizeSql(sql)) {
    out += sql.slice(cursor, token.from)
    if (token.kind !== "comment") out += token.text
    cursor = token.to
  }
  return out + sql.slice(cursor)
}

/** Whether there is anything here to run or estimate. */
export function hasStatement(sql: string): boolean {
  return stripSqlComments(sql).trim().length > 0
}

/** First line of a statement, for naming things after it. */
export function sqlSummary(sql: string, max = 60): string {
  const line = stripSqlComments(sql).trim().split("\n")[0]?.trim() ?? ""
  return line.length > max ? `${line.slice(0, max - 1)}…` : line
}

export type SqlStatement = {
  /** The statement's text: the slice of the input between `from` and `to`. */
  text: string
  /** The statement's start offset in the input. */
  from: number
  /** The statement's end offset in the input, exclusive. */
  to: number
}

/**
 * Split a buffer into the statements it holds, on the semicolons that sit
 * outside quotes and comments.
 *
 * A statement's range starts at its first token that is not a comment and
 * ends at its last, so the comments around it stay out of the text that
 * gets run. A piece that holds nothing but comments has no statement.
 */
export function splitStatements(sql: string): SqlStatement[] {
  const statements: SqlStatement[] = []
  let first: number | null = null
  let last = 0
  for (const token of tokenizeSql(sql)) {
    if (token.kind === "punctuation" && token.text === ";") {
      if (first !== null) statements.push({ text: sql.slice(first, last), from: first, to: last })
      first = null
      continue
    }
    if (token.kind === "comment") continue
    if (first === null) first = token.from
    last = token.to
  }
  if (first !== null) statements.push({ text: sql.slice(first, last), from: first, to: last })
  return statements
}
