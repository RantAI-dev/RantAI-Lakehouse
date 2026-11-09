import { isIcebergCandidate } from "@/lib/lakehouse-view"
import { pinSnapshot } from "./snapshot-picker"
import type { AssetDetail } from "@/services/contracts/assets"
import type { LakehouseTableDetail } from "@/services/contracts/lakehouse"
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
 * The ClickHouse target for this asset: the name the API resolved when it
 * sent one — the table it proved the console can read, the ClickHouse
 * `DataLakeCatalog` name for a Bronze table, the Silver name for a stand-in
 * — else the `namespace.name` form Query Studio's autocomplete offers,
 * quoted where needed.
 *
 * It stands apart from `assetQueryTarget`, which answers Trino for an
 * Iceberg candidate: a catalog link may only name an Iceberg table the page
 * has seen (owner, 2026-11-08).
 */
function assetClickHouseTarget(a: Target): {
  engine: QueryEngine
  table: string
} {
  if (a.queryTarget) return a.queryTarget
  return { engine: "clickhouse", table: `${quoteIdent(a.namespace)}.${quoteIdent(a.name)}` }
}

/**
 * Where Query Studio opens this asset, and what to call it there.
 *
 * A Bronze table opens on a Trino tab with `iceberg.bronze.<table>` only
 * when its Iceberg table is proven to exist: `table` is the detail the page
 * loaded, and `null` while it loads, after it failed, and when the table is
 * not there at all. The registry's `tableName` says a Bronze table might
 * exist, not that it does — a Trino tab on a table that is not there
 * answers "table does not exist" (owner, 2026-11-08; #409 opened the link
 * for every candidate).
 *
 * Without that proof the link is the ClickHouse one: the name the API
 * resolved, else the `namespace.name` form. A Silver stand-in keeps the
 * API's ClickHouse name: Trino has no table of its own for it.
 */
export function assetStudioTarget(
  a: Target & Pick<AssetDetail, "tableKey">,
  table: LakehouseTableDetail | null
): {
  engine: QueryEngine
  table: string
} {
  if (isIcebergCandidate(a) && a.tableName && assetStandInTable(a) === null && table !== null) {
    return { engine: "trino", table: trinoBronzeName(a.tableName) }
  }
  return assetClickHouseTarget(a)
}

/**
 * A starter `SELECT` for this asset, with explicit columns when there are
 * few. It reads the table the studio link opens, so it names the same one.
 */
export function assetStarterSql(
  a: Target & Pick<AssetDetail, "schema" | "tableKey">,
  table: LakehouseTableDetail | null
) {
  const cols =
    a.schema.length > 0 && a.schema.length <= SNIPPET_MAX_COLUMNS
      ? a.schema.map((c) => `  ${quoteIdent(c.name)}`).join(",\n")
      : "  *"
  return `SELECT\n${cols}\nFROM ${assetStudioTarget(a, table).table}\nLIMIT 100`
}

/** Query Studio, opened on this asset's starter query and the engine it needs. */
export function assetQueryStudioHref(
  a: Target & Pick<AssetDetail, "schema" | "tableKey">,
  table: LakehouseTableDetail | null
) {
  const params = new URLSearchParams({
    sql: assetStarterSql(a, table),
    engine: assetStudioTarget(a, table).engine,
  })
  return `/query-studio?${params.toString()}`
}

/**
 * Query Studio, opened on a Trino tab with the starter query pinned to one
 * Iceberg snapshot, the table as it was after that load. `null` when the
 * asset is not an Iceberg candidate, when its Iceberg table is not there
 * (a version of a table that does not exist is nothing), when its rows do
 * not open on Trino (a Silver stand-in has no snapshots to go back to), or
 * when the id is not the plain number Iceberg issues.
 *
 * `pinSnapshot` writes the pin, so the two callers of it cannot drift. The
 * pin is on the table's own reference, the one `FOR VERSION AS OF` belongs
 * to (`DATA-21` D5). Past this link the API reads that clause on Trino
 * (`DATA-21`), which is why it is no longer withheld as it was while
 * `DATA-16` F6 refused it.
 */
export function assetSnapshotQueryHref(
  a: Target & Pick<AssetDetail, "schema" | "tableKey">,
  snapshotId: string,
  table: LakehouseTableDetail | null
): string | null {
  if (!isIcebergCandidate(a)) return null
  const target = assetStudioTarget(a, table)
  if (target.engine !== "trino") return null
  const sql = pinSnapshot(assetStarterSql(a, table), "trino", snapshotId, target.table)
  if (sql === null) return null
  const params = new URLSearchParams({ sql, engine: target.engine })
  return `/query-studio?${params.toString()}`
}
