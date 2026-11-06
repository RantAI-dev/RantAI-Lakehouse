import { isIcebergCandidate } from "@/lib/lakehouse-view"
import { pinSnapshot } from "./snapshot-picker"
import type { AssetDetail } from "@/services/contracts/assets"
import type { QueryEngine } from "@/services/contracts/queries"

/**
 * Past this many columns the snippet falls back to `*`: a 40-line column
 * list is not something anyone copies to "just look at the data".
 */
const SNIPPET_MAX_COLUMNS = 12

/** The fields that decide where an asset is queried. */
type Target = Pick<AssetDetail, "layer" | "type" | "tableName" | "namespace" | "name" | "queryTarget">

/** A simple identifier needs no quoting; anything else (spaces, dashes) does. */
export function quoteIdent(part: string) {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(part) ? part : `"${part.replace(/"/g, '""')}"`
}

/**
 * A Bronze table's name in Trino: `iceberg.bronze.<table>`.
 *
 * A table name that is not a plain identifier comes out quoted
 * (`iceberg.bronze."order lines"`). Completion reads such a name only up to
 * the quote, so an alias on it does not resolve; plain names are the
 * supported case.
 */
export function trinoBronzeName(tableName: string): string {
  return `iceberg.bronze.${quoteIdent(tableName)}`
}

/**
 * Where a starter query for this asset should run, and what to call it
 * there. The API's own `queryTarget` wins whenever it sent one — it knows
 * which table actually exists (an Iceberg-only dataset is read through
 * ClickHouse's `DataLakeCatalog` database). Without it: an Iceberg
 * candidate is addressed through Trino's `iceberg` catalog
 * (`iceberg.bronze.<table>`), and anything else keeps the `namespace.name`
 * form Query Studio's autocomplete offers, quoted where needed (double
 * quotes, which both ClickHouse and Trino accept).
 */
export function assetQueryTarget(a: Target): {
  engine: QueryEngine
  table: string
} {
  if (a.queryTarget) return a.queryTarget
  if (isIcebergCandidate(a) && a.tableName) {
    return { engine: "trino", table: trinoBronzeName(a.tableName) }
  }
  return { engine: "clickhouse", table: `${quoteIdent(a.namespace)}.${quoteIdent(a.name)}` }
}

/**
 * The table an asset's rows are read from when that is not the asset's
 * own: a Bronze dataset whose Bronze table cannot be read is shown through
 * its Silver model (`routes::catalog_source`). `null` when the page reads
 * the asset itself, or when the API named no table at all.
 */
export function assetStandInTable(a: Pick<AssetDetail, "queryTarget" | "tableKey">): string | null {
  const read = a.queryTarget?.policyTable
  return read && a.tableKey && read !== a.tableKey ? read : null
}

/**
 * Where Query Studio opens this asset, and what to call it there.
 *
 * A Bronze table that Trino reads opens on a Trino tab with
 * `iceberg.bronze.<table>`, the name the schema browser offers on that tab.
 * The API's `queryTarget` names the table for the console's own reads (for
 * a Bronze table, the ClickHouse `DataLakeCatalog` name), so it does not
 * decide this. A Silver stand-in keeps the API's ClickHouse name: Trino
 * has no table of its own for it. Anything else follows `assetQueryTarget`.
 */
export function assetStudioTarget(a: Target & Pick<AssetDetail, "tableKey">): {
  engine: QueryEngine
  table: string
} {
  if (isIcebergCandidate(a) && a.tableName && assetStandInTable(a) === null) {
    return { engine: "trino", table: trinoBronzeName(a.tableName) }
  }
  return assetQueryTarget(a)
}

/** A starter `SELECT` for this asset, with explicit columns when there are few. */
export function assetStarterSql(a: Target & Pick<AssetDetail, "schema" | "tableKey">) {
  const cols =
    a.schema.length > 0 && a.schema.length <= SNIPPET_MAX_COLUMNS
      ? a.schema.map((c) => `  ${quoteIdent(c.name)}`).join(",\n")
      : "  *"
  return `SELECT\n${cols}\nFROM ${assetStudioTarget(a).table}\nLIMIT 100`
}

/** Query Studio, opened on this asset's starter query and the engine it needs. */
export function assetQueryStudioHref(a: Target & Pick<AssetDetail, "schema" | "tableKey">) {
  const params = new URLSearchParams({
    sql: assetStarterSql(a),
    engine: assetStudioTarget(a).engine,
  })
  return `/query-studio?${params.toString()}`
}

/**
 * Query Studio, opened on the starter query pinned to one Iceberg snapshot
 * — the table as it was after that load. `null` when the asset is not
 * read from its Iceberg table (a `silver.*` read has no snapshots to go
 * back to), or when the id is not the plain number Iceberg issues.
 *
 * `pinSnapshot` writes the pin, so the two callers of it cannot drift. A
 * Bronze table opens on Trino and is pinned on its own reference, the one
 * `FOR VERSION AS OF` belongs to (`DATA-21` D5). Past this link the API
 * reads that clause on Trino (`DATA-21`), which is why it is no longer
 * withheld as it was while `DATA-16` F6 refused it. A row that keeps its
 * ClickHouse name is pinned with the query-wide setting.
 */
export function assetSnapshotQueryHref(
  a: Target & Pick<AssetDetail, "schema" | "tableKey">,
  snapshotId: string
): string | null {
  if (!isIcebergCandidate(a)) return null
  const target = assetStudioTarget(a)
  let sql: string | null
  if (target.engine === "trino") {
    sql = pinSnapshot(assetStarterSql(a), "trino", snapshotId, target.table)
  } else {
    // The `DataLakeCatalog` name, e.g. icecat_api.`bronze.orders`.
    if (!target.table.includes("`bronze.")) return null
    sql = pinSnapshot(assetStarterSql(a), target.engine, snapshotId)
  }
  if (sql === null) return null
  const params = new URLSearchParams({ sql, engine: target.engine })
  return `/query-studio?${params.toString()}`
}
