import { formatDateTime } from "./format"
import { msToIso } from "./lakehouse-view"
import type { LakehouseSnapshot } from "@/services/contracts/lakehouse"

/**
 * Query Studio's snapshot picker (WS2 §4) and the asset page's snapshot
 * links both write the same thing: the ClickHouse query setting
 * `iceberg_snapshot_id`. `DATA-16` F5/F6: the picker used to write Trino's
 * `FOR VERSION AS OF`, which the API cannot parse and refuses for every
 * user; one function now writes the pin, per engine, so the two callers
 * cannot drift apart.
 *
 * The setting is query-wide: it applies to every Iceberg table the query
 * reads (`DATA-16` F7), which is why the picker no longer names a table in
 * the SQL.
 *
 * `snapshotId` is kept a string end to end and never passed through
 * `Number`/`parseInt`/arithmetic: Iceberg snapshot ids routinely exceed
 * `Number.MAX_SAFE_INTEGER`, and the API sends them as strings on purpose
 * (see `contracts/lakehouse.ts`'s `LakehouseSnapshot.id`) so a large id's
 * digits are never silently rounded. It is also checked to be digits only,
 * because it is interpolated into SQL.
 *
 * Returns `null` for any engine but ClickHouse (Trino time travel is
 * `DATA-21`) and for an id that is not a plain number.
 */
export function pinSnapshot(sql: string, engine: string, snapshotId: string): string | null {
  if (engine !== "clickhouse" || !/^\d+$/.test(snapshotId)) return null
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
