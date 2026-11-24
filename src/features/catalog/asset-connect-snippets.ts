/**
 * Pure builders for the snippets shown in a catalog asset's "Connect" dialog
 * (task `reczz28d1eZjrBAZ`).
 *
 * Two doors, and only two, are offered here:
 *
 * - The governed query API (`POST /api/query/run`). It is available for every
 *   zone; the server applies the caller's column masks, row filters and audit
 *   event on the way through.
 * - The Iceberg REST catalog door (`/api/iceberg/v1`), for a published Gold
 *   table only. The API forwards reads under its own catalog principal and
 *   counts them.
 *
 * There is deliberately NO raw ClickHouse snippet here. A `:8123`/`:9000`
 * JDBC or native connection would go around masking, RLS and audit entirely
 * — which is the reason these two doors exist. Keep that rule in the tests
 * as well as in this file.
 */

/** The governed query route, exactly as `routes::query::run` registers it. */
export const QUERY_RUN_PATH = "/api/query/run"

/** The Iceberg REST door's route prefix (`routes::iceberg_door::router`). */
export const ICEBERG_REST_PATH = "/api/iceberg/v1"

/** The client base URI. Iceberg clients append `/v1` to this themselves. */
export const ICEBERG_CLIENT_BASE = "/api/iceberg"

/** Verified in `src/app/(admin)/admin/service-identities/page.tsx`. */
export const SERVICE_IDENTITIES_HREF = "/admin/service-identities"

/** Placeholders — never a real deployment address, key or hostname. */
export const LAKEHOUSE_URL_PLACEHOLDER = "<lakehouse-url>"
export const QUERY_TOKEN_PLACEHOLDER = "<service-token>"
export const ICEBERG_TOKEN_PLACEHOLDER = "<iceberg-token>"

/** The one warehouse the door serves (`LAKEKEEPER_WAREHOUSE` in this stack). */
export const DEFAULT_WAREHOUSE = "default"

/** The one namespace outside readers may touch (ADR 0016 point 2). */
export const GOLD_NAMESPACE = "gold"

/** Plain-language governance note, shown under the REST snippet. */
export const GOVERNED_API_NOTE =
  "Queries through the governed REST API enforce your column masking, " +
  "row-level security (RLS) and audit logging on every request."

/** Which token the REST snippet needs, and where to get one. */
export const QUERY_TOKEN_NOTE =
  "Use a service token with the query:read scope. Create one in Settings › Service Identities."

/** Why a whole-table Iceberg scan over-reports, and what to do about it. */
export const EXPORTED_AT_NOTE =
  "Each publish adds a full copy, so an unfiltered scan counts the same rows " +
  "more than once. Until DATA-6 / DATA-3 land, filter on `_exported_at` to keep " +
  "one export; where a `<table>_latest` companion is published, it holds only " +
  "the newest copy and needs no filter."

/**
 * The mart name behind a Gold asset id. The catalog names a serving mart
 * `serving.<mart>`, while the Gold export routes name it `<mart>` — the same
 * trim the publish card uses, kept in one place so a change to that
 * convention cannot leave the connect snippets pointing at a wrong table.
 */
export function martFromAssetId(assetId: string): string {
  return assetId.startsWith("serving.") ? assetId.slice("serving.".length) : assetId
}

/** A ClickHouse identifier part is safe to leave bare only when it is a word. */
const PLAIN_IDENTIFIER = /^[A-Za-z_][A-Za-z0-9_]*$/

/**
 * Quote one part of a ClickHouse table identifier. A hyphen, space or quote
 * in an asset id must never turn into SQL syntax in a copied snippet.
 */
function quoteIdentifierPart(part: string): string {
  if (PLAIN_IDENTIFIER.test(part)) return part
  return `\`${part.replace(/`/g, "``")}\``
}

/**
 * The fully-qualified `<database>.<table>` identifier a governed SQL query
 * names. The registry's asset id is exactly that shape (`silver.orders`,
 * `serving.mart_a`). A malformed id is refused rather than pasted into a
 * snippet that would not run.
 */
export function governedSqlIdentifier(qualifiedTable: string): string {
  const parts = qualifiedTable.split(".")
  if (parts.length !== 2 || parts.some((part) => part.length === 0)) {
    throw new Error("A governed table id must be <zone>.<table>.")
  }
  return parts.map(quoteIdentifierPart).join(".")
}

/** What the REST-snippet builder needs: the asset's fully-qualified id. */
export type RestSnippetInput = {
  qualifiedTable: string
}

/**
 * The `curl` for `POST /api/query/run`. The body is the one `SqlBody` the
 * route deserialises: `sql` plus an explicit `engine` (the route defaults to
 * `clickhouse` when it is absent, but showing it makes the snippet honest
 * about which engine ran). `JSON.stringify` does the escaping, so a quoted
 * identifier can never break the JSON.
 */
export function buildRestApiSnippet(input: RestSnippetInput): string {
  const sql = `SELECT * FROM ${governedSqlIdentifier(input.qualifiedTable)} LIMIT 100`
  const body = JSON.stringify({ sql, engine: "clickhouse" })
  return [
    `curl -X POST '${LAKEHOUSE_URL_PLACEHOLDER}${QUERY_RUN_PATH}' \\`,
    `  -H 'Content-Type: application/json' \\`,
    `  -H 'Authorization: Bearer ${QUERY_TOKEN_PLACEHOLDER}' \\`,
    `  -d '${body}'`,
  ].join("\n")
}

/** What the three Iceberg-snippet builders need. */
export type IcebergSnippetInput = {
  /** Iceberg namespace, `gold` for the published copy. */
  namespace: string
  /** The published table's name, without the namespace prefix. */
  table: string
  /** The deployment's warehouse; the door serves `default` only. */
  warehouse?: string
}

/** The three client snippets, one per sub-tab of the Iceberg tab. */
export type IcebergSnippets = {
  duckdb: string
  pyiceberg: string
  spark: string
}

/**
 * The three `/api/iceberg/v1` client snippets.
 *
 * Each client is given the base URI `${LAKEHOUSE_URL_PLACEHOLDER}/api/iceberg`
 * because pyiceberg, DuckDB and Spark each append `/v1/...` themselves — the
 * routes the door actually serves are under `/api/iceberg/v1`. Passing the
 * path with `/v1` as the client's `uri` would produce `/v1/v1/config`.
 * The comment in each snippet says this, so a reader does not "fix" it.
 */
export function buildIcebergSnippets(input: IcebergSnippetInput): IcebergSnippets {
  const warehouse = input.warehouse ?? DEFAULT_WAREHOUSE
  const uri = `${LAKEHOUSE_URL_PLACEHOLDER}${ICEBERG_CLIENT_BASE}`
  const qualified = `${input.namespace}.${input.table}`

  const pyiceberg = [
    `# Iceberg REST door: endpoints under ${ICEBERG_REST_PATH}.`,
    `# Token from a service identity scoped iceberg:read.`,
    `from pyiceberg.catalog import load_catalog`,
    ``,
    `catalog = load_catalog(`,
    `    "rest",`,
    `    uri="${uri}",  # the client appends /v1 to this base`,
    `    token="${ICEBERG_TOKEN_PLACEHOLDER}",`,
    `    warehouse="${warehouse}",`,
    `)`,
    `table = catalog.load_table(("${input.namespace}", "${input.table}"))`,
    `print(table.scan().to_arrow().num_rows)`,
  ].join("\n")

  const duckdb = [
    `-- Iceberg REST door: endpoints under ${ICEBERG_REST_PATH}.`,
    `-- Token from a service identity scoped iceberg:read.`,
    `INSTALL iceberg;`,
    `LOAD iceberg;`,
    ``,
    `ATTACH 'lakehouse' AS cat (`,
    `    TYPE iceberg,`,
    `    URI '${uri}',  -- the client appends /v1 to this base`,
    `    AUTHORIZATION_TYPE 'OAUTH2',`,
    `    TOKEN '${ICEBERG_TOKEN_PLACEHOLDER}'`,
    `);`,
    ``,
    `SELECT count(*) FROM cat.${qualified};`,
  ].join("\n")

  const spark = [
    `# Iceberg REST door: endpoints under ${ICEBERG_REST_PATH}.`,
    `# Token from a service identity scoped iceberg:read.`,
    `from pyspark.sql import SparkSession`,
    ``,
    `spark = (`,
    `    SparkSession.builder`,
    `    .config("spark.sql.extensions", "org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions")`,
    `    .config("spark.sql.catalog.lakehouse", "org.apache.iceberg.spark.SparkCatalog")`,
    `    .config("spark.sql.catalog.lakehouse.catalog-impl", "org.apache.iceberg.rest.RESTCatalog")`,
    `    .config("spark.sql.catalog.lakehouse.uri", "${uri}")`,
    `    .config("spark.sql.catalog.lakehouse.token", "${ICEBERG_TOKEN_PLACEHOLDER}")`,
    `    .config("spark.sql.catalog.lakehouse.warehouse", "${warehouse}")`,
    `    .getOrCreate()`,
    `)`,
    `spark.sql("SELECT * FROM lakehouse.${qualified} LIMIT 100").show()`,
  ].join("\n")

  return { duckdb, pyiceberg, spark }
}
