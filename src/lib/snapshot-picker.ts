import { formatDateTime } from "./format"
import { msToIso } from "./lakehouse-view"
import type { LakehouseSnapshot } from "@/services/contracts/lakehouse"

/**
 * Query Studio's snapshot picker (WS2 §4) and the asset page's snapshot
 * links both write the same thing: the pin that holds a query at one
 * version of a table. `DATA-16` F5/F6: the picker used to write Trino's
 * `FOR VERSION AS OF`, which the API could not parse and refused for
 * every user; one function now writes the pin, per engine, so the two
 * callers cannot drift apart.
 *
 * On ClickHouse the pin is the query-wide setting `iceberg_snapshot_id`:
 * it applies to every Iceberg table the query reads (`DATA-16` F7), which
 * is why the picker does not name a table in the SQL.
 *
 * On Trino a version belongs to ONE table (`DATA-21` D5), so the pin is
 * written on that table's own reference — `FROM bronze.orders FOR VERSION
 * AS OF 123`. `tableRef` is the reference the caller picked
 * (`namespace.table`); a statement that does not read it cannot be pinned,
 * and `null` says so.
 *
 * `snapshotId` is kept a string end to end and never passed through
 * `Number`/`parseInt`/arithmetic: Iceberg snapshot ids routinely exceed
 * `Number.MAX_SAFE_INTEGER`, and the API sends them as strings on purpose
 * (see `contracts/lakehouse.ts`'s `LakehouseSnapshot.id`) so a large id's
 * digits are never silently rounded. It is also checked to be digits only,
 * because it is interpolated into SQL.
 *
 * Returns `null` for an engine that has no pin (an engine other than
 * ClickHouse or Trino), for an id that is not a plain number, and on Trino
 * for a statement that does not read the table the caller picked.
 */
export function pinSnapshot(
  sql: string,
  engine: string,
  snapshotId: string,
  tableRef?: string
): string | null {
  if (!/^\d+$/.test(snapshotId)) return null
  if (engine === "trino") return pinSnapshotOnTrino(sql, snapshotId, tableRef)
  if (engine !== "clickhouse") return null
  const setting = `iceberg_snapshot_id = ${snapshotId}`

  const clause = lastTopLevelSettings(sql)
  if (clause === null) {
    const body = sql.replace(/[\s;]+$/, "")
    return `${body}\nSETTINGS ${setting}`
  }
  const head = sql.slice(0, clause)
  const tail = sql.slice(clause + "SETTINGS".length)
  // Comments carry no meaning in the list, and one left in place could
  // swallow the setting appended after it.
  const noise = blank(tail, true)
  const items: string[] = []
  let depth = 0
  let from = 0
  const commentless = blank(tail, false)
  for (let i = 0; i <= noise.length; i++) {
    const ch = noise[i]
    if (ch === "(") depth++
    else if (ch === ")") depth--
    else if (i === noise.length || (ch === "," && depth === 0)) {
      items.push(commentless.slice(from, i).replace(/[\s;]+$/, "").trim())
      from = i + 1
    }
  }
  const kept = items.filter((it) => it !== "" && !/^iceberg_snapshot_id\s*=/i.test(it))
  return `${head}SETTINGS ${[...kept, setting].join(", ")}`
}

/**
 * The Trino pin: `FOR VERSION AS OF <id>` written after the table's own
 * reference in the statement (`DATA-21` D5). An earlier pin on the same
 * table is REPLACED, not stacked — the same rule the ClickHouse form
 * follows, and what the user expects when they pick another version.
 *
 * The reference is matched on identifier boundaries, so `bronze.orders`
 * matches inside a catalog-qualified `iceberg.bronze.orders` and never
 * matches a longer name that shares its prefix (`bronze.orders_archive`).
 * Each part may be double-quoted (`"bronze"."orders"`), the way `Trino`
 * writes a quoted identifier.
 */
function pinSnapshotOnTrino(
  sql: string,
  snapshotId: string,
  tableRef: string | undefined
): string | null {
  if (!tableRef) return null
  const parts = tableRef.split(".").map((part) => {
    const escaped = part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
    return `"?${escaped}"?`
  })
  const pattern = new RegExp(`(?<!\\w)${parts.join("\\s*\\.\\s*")}(?!\\w)`, "g")
  // The same statement with its comments and quoted runs blanked: a
  // reference inside one of them is not a table the query reads.
  const noise = blank(sql, true)

  let match: RegExpExecArray | null
  while ((match = pattern.exec(sql)) !== null) {
    // A match that starts inside a blanked run is inside a comment, a
    // literal or a quoted identifier — the ClickHouse-style
    // `` `bronze.orders` `` name is one of those. A match that STARTS
    // with a quote is the `Trino` form itself, so it is kept.
    if (noise[match.index] !== sql[match.index] && sql[match.index] !== '"') continue
    const end = match.index + match[0].length
    const rest = sql.slice(end)
    const pinned = /^\s*FOR\s+VERSION\s+AS\s+OF\s+\d+/i.exec(rest)
    if (pinned) {
      const after = rest.slice(pinned[0].length)
      return `${sql.slice(0, end)} FOR VERSION AS OF ${snapshotId}${after}`
    }
    return `${sql.slice(0, end)} FOR VERSION AS OF ${snapshotId}${rest}`
  }
  return null
}

/**
 * Start index of the last `SETTINGS` keyword that sits outside every
 * string, quoted identifier, comment and parenthesis, or `null`. A textual
 * scan, not a SQL parser: the result is ordinary text in the editor that
 * the user can still edit.
 */
function lastTopLevelSettings(sql: string): number | null {
  const noise = blank(sql, true)
  let depth = 0
  let found: number | null = null
  for (let i = 0; i < noise.length; i++) {
    const ch = noise[i]
    if (ch === "(") depth++
    else if (ch === ")") depth--
    else if (depth === 0 && /[Ss]/.test(ch) && /^settings(?![\w$])/i.test(noise.slice(i, i + 9))) {
      if (i === 0 || !/[\w$]/.test(noise[i - 1])) found = i
    }
  }
  return found
}

/**
 * Same-length copy of `sql` with comments blanked and, when `literals` is
 * true, also string literals and quoted identifiers (so a `SETTINGS`, `,`
 * or `(` inside one is invisible). Backslash escapes and doubled quotes
 * are ClickHouse's.
 */
function blank(sql: string, literals: boolean): string {
  let out = ""
  let i = 0
  const pad = (s: string) => s.replace(/[^\n]/g, " ")
  while (i < sql.length) {
    const two = sql.slice(i, i + 2)
    if (two === "--" || sql[i] === "#") {
      const end = sql.indexOf("\n", i)
      const stop = end === -1 ? sql.length : end
      out += pad(sql.slice(i, stop))
      i = stop
    } else if (two === "/*") {
      const end = sql.indexOf("*/", i + 2)
      const stop = end === -1 ? sql.length : end + 2
      out += pad(sql.slice(i, stop))
      i = stop
    } else if (sql[i] === "'" || sql[i] === '"' || sql[i] === "`") {
      const quote = sql[i]
      let j = i + 1
      while (j < sql.length && sql[j] !== quote) j += sql[j] === "\\" ? 2 : 1
      const stop = Math.min(j + 1, sql.length)
      out += literals ? pad(sql.slice(i, stop)) : sql.slice(i, stop)
      i = stop
    } else {
      out += sql[i]
      i++
    }
  }
  return out
}

/**
 * What the snapshot list says about how far back versions go
 * (`DATA-16` D2): the count and the oldest date, read from the list
 * itself. No number of days is promised, because expiry is a maintenance
 * setting that can change.
 */
export function snapshotRetentionText(snapshots: LakehouseSnapshot[]): string {
  if (snapshots.length === 0) return "No versions yet."
  const oldest = Math.min(...snapshots.map((s) => s.timestampMs))
  const when = formatDateTime(msToIso(oldest))
  return snapshots.length === 1
    ? `1 version, from ${when}`
    : `${snapshots.length} versions, the oldest from ${when}`
}
