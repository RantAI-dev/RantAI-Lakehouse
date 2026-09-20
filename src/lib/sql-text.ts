/**
 * Reading SQL text well enough to decide what the UI should do with it —
 * not to execute it. The server has its own guard (`routes/query.rs`);
 * this only stops the console from asking pointless questions about a
 * buffer that holds nothing but comments.
 */

/** `sql` with line comments (`--`, `#`) and block comments removed. */
export function stripSqlComments(sql: string): string {
  let out = ""
  let i = 0
  while (i < sql.length) {
    const two = sql.slice(i, i + 2)
    if (two === "--" || sql[i] === "#") {
      const end = sql.indexOf("\n", i)
      i = end === -1 ? sql.length : end
      continue
    }
    if (two === "/*") {
      const end = sql.indexOf("*/", i + 2)
      i = end === -1 ? sql.length : end + 2
      continue
    }
    out += sql[i]
    i += 1
  }
  return out
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
