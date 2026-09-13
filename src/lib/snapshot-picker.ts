/**
 * `insertAsOfClause` is a best-effort textual insertion into a SQL string,
 * not a SQL parser — Query Studio's snapshot picker (WS2 §4) uses it to
 * drop a Trino time-travel clause after a table reference the user has
 * already typed. The result is ordinary text in the editor: the user can
 * still edit it freely, move it, or delete it.
 *
 * It only ever inserts `FOR VERSION AS OF <snapshotId>`, never
 * `FOR TIMESTAMP AS OF`: Trino requires a typed `TIMESTAMP` literal for the
 * latter (`TIMESTAMP '2026-01-01 00:00:00 UTC'`) and rejects the ISO
 * `T...Z` form outright, so there is no safe generic timestamp string to
 * insert. A snapshot id is a plain Trino `bigint` — no quoting, no time
 * zone — so it is always safe to interpolate.
 *
 * `snapshotId` is kept a string end to end and never passed through
 * `Number`/`parseInt`/arithmetic: Iceberg snapshot ids routinely exceed
 * `Number.MAX_SAFE_INTEGER`, and the API sends them as strings on purpose
 * (see `contracts/lakehouse.ts`'s `LakehouseSnapshot.id`) so a large id's
 * digits are never silently rounded.
 */
export function insertAsOfClause(sql: string, tableRef: string, snapshotId: string): string {
  const escaped = tableRef.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
  // Identifier-boundary matching on `\w` only (not `.`): this lets
  // `bronze.orders` match inside a catalog-qualified `iceberg.bronze.orders`
  // reference (the char before the match is `.`, not a word char) while
  // still refusing to match a longer table name sharing the same prefix,
  // like `bronze.orders_archive` (the char after the match is `_`, a word
  // char, so the lookahead fails).
  const pattern = new RegExp(`(?<!\\w)${escaped}(?!\\w)`, "g")

  let match: RegExpExecArray | null
  while ((match = pattern.exec(sql)) !== null) {
    const end = match.index + match[0].length

    if (isInsideSingleQuotedString(sql, match.index)) continue

    const rest = sql.slice(end)
    if (/^\s+FOR\s+(VERSION|TIMESTAMP)\s+AS\s+OF\b/i.test(rest)) continue

    return `${sql.slice(0, end)} FOR VERSION AS OF ${snapshotId}${sql.slice(end)}`
  }

  return sql
}

/** Odd count of unescaped `'` before `index` means `index` sits inside a string literal. */
function isInsideSingleQuotedString(sql: string, index: number): boolean {
  let insideQuote = false
  for (let i = 0; i < index; i++) {
    if (sql[i] === "'") insideQuote = !insideQuote
  }
  return insideQuote
}
