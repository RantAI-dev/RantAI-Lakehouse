import { isIcebergCandidate } from "@/lib/lakehouse-view"
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
function quoteIdent(part: string) {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(part) ? part : `"${part.replace(/"/g, '""')}"`
}

/**
 * Where a starter query for this asset should run, and what to call it
 * there. The API's own `queryTarget` wins whenever it sent one — it knows
 * which table actually exists (an Iceberg-only dataset is read through
 * ClickHouse's `DataLakeCatalog` database). Without it: an Iceberg
 * candidate is addressed through Trino's `iceberg` catalog, the same
 * `iceberg.bronze.<table>` form Query Studio's time-travel control writes,
 * and anything else keeps the `namespace.name` form Query Studio's
 * autocomplete offers, quoted where needed (double quotes, which both
 * ClickHouse and Trino accept).
 */
export function assetQueryTarget(a: Target): {
  engine: QueryEngine
  table: string
} {
  if (a.queryTarget) return a.queryTarget
  if (isIcebergCandidate(a) && a.tableName) {
    return { engine: "trino", table: `iceberg.bronze.${quoteIdent(a.tableName)}` }
  }
  return { engine: "clickhouse", table: `${quoteIdent(a.namespace)}.${quoteIdent(a.name)}` }
}

/** A starter `SELECT` for this asset, with explicit columns when there are few. */
export function assetStarterSql(a: Target & Pick<AssetDetail, "schema">) {
  const cols =
    a.schema.length > 0 && a.schema.length <= SNIPPET_MAX_COLUMNS
      ? a.schema.map((c) => `  ${quoteIdent(c.name)}`).join(",\n")
      : "  *"
  return `SELECT\n${cols}\nFROM ${assetQueryTarget(a).table}\nLIMIT 100`
}

/** Query Studio, opened on this asset's starter query and the engine it needs. */
export function assetQueryStudioHref(a: Target & Pick<AssetDetail, "schema">) {
  const params = new URLSearchParams({
    sql: assetStarterSql(a),
    engine: assetQueryTarget(a).engine,
  })
  return `/query-studio?${params.toString()}`
}
