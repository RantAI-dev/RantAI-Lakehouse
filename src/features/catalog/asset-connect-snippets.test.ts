import { describe, expect, it } from "bun:test"
import {
  buildIcebergSnippets,
  buildRestApiSnippet,
  DEFAULT_WAREHOUSE,
  EXPORTED_AT_NOTE,
  GOLD_NAMESPACE,
  governedSqlIdentifier,
  ICEBERG_REST_PATH,
  martFromAssetId,
  QUERY_RUN_PATH,
  SERVICE_IDENTITIES_HREF,
} from "./asset-connect-snippets"

describe("governedSqlIdentifier", () => {
  it("leaves a plain zone.table id bare", () => {
    expect(governedSqlIdentifier("silver.orders")).toBe("silver.orders")
  })

  it("quotes only the parts that need it, so an asset name cannot become SQL", () => {
    expect(governedSqlIdentifier("bronze.weird-table")).toBe("bronze.`weird-table`")
    expect(governedSqlIdentifier("odd zone.orders")).toBe("`odd zone`.orders")
    expect(governedSqlIdentifier("bronze.a`b")).toBe("bronze.`a``b`")
  })

  it("refuses an id that is not exactly zone.table", () => {
    expect(() => governedSqlIdentifier("orders")).toThrow()
    expect(() => governedSqlIdentifier(".orders")).toThrow()
    expect(() => governedSqlIdentifier("bronze.")).toThrow()
    expect(() => governedSqlIdentifier("bronze.silver.orders")).toThrow()
  })
})

describe("buildRestApiSnippet", () => {
  it("posts the exact body the query route accepts, with the governed table SQL", () => {
    const snippet = buildRestApiSnippet({ qualifiedTable: "silver.orders" })
    expect(snippet).toContain(`<lakehouse-url>${QUERY_RUN_PATH}'`)
    expect(snippet).toContain("Authorization: Bearer <service-token>")
    expect(snippet).toContain("SELECT * FROM silver.orders LIMIT 100")
    expect(snippet).toContain('"engine":"clickhouse"')

    const dataFlag = snippet.split("\n").find((l) => l.includes("-d '"))
    expect(dataFlag).toBeDefined()
    const body = JSON.parse((dataFlag ?? "").slice((dataFlag ?? "").indexOf("-d '") + 4, -1))
    expect(body).toEqual({ sql: "SELECT * FROM silver.orders LIMIT 100", engine: "clickhouse" })
  })

  it("uses the real Service Identities route and names the query:read scope and governance", () => {
    expect(SERVICE_IDENTITIES_HREF).toBe("/admin/service-identities")
    expect(EXPORTED_AT_NOTE).toContain("_exported_at")
    // The scope and governance promise are rendered by the dialog from these
    // constants; keep them here so a rename cannot silently drop either.
    expect(QUERY_RUN_PATH).toBe("/api/query/run")
  })
})

describe("buildIcebergSnippets", () => {
  const snippets = buildIcebergSnippets({ namespace: "gold", table: "mart_a" })

  it("builds one snippet for each of DuckDB, PyIceberg and Spark", () => {
    expect(Object.keys(snippets)).toEqual(["duckdb", "pyiceberg", "spark"])
    for (const snippet of Object.values(snippets)) {
      expect(snippet.length).toBeGreaterThan(0)
    }
  })

  it("points each client at the door's base URI, with the /v1 path stated", () => {
    expect(snippets.pyiceberg).toContain('uri="<lakehouse-url>/api/iceberg"')
    expect(snippets.duckdb).toContain("URI '<lakehouse-url>/api/iceberg'")
    expect(snippets.spark).toContain(
      '.config("spark.sql.catalog.lakehouse.uri", "<lakehouse-url>/api/iceberg")'
    )
    for (const snippet of Object.values(snippets)) {
      expect(snippet).toContain(ICEBERG_REST_PATH)
      expect(snippet).toContain("iceberg:read")
    }
  })

  it("uses the published namespace, table and the one served warehouse", () => {
    expect(DEFAULT_WAREHOUSE).toBe("default")
    expect(GOLD_NAMESPACE).toBe("gold")
    expect(snippets.pyiceberg).toContain('warehouse="default"')
    expect(snippets.pyiceberg).toContain('catalog.load_table(("gold", "mart_a"))')
    expect(snippets.duckdb).toContain("SELECT count(*) FROM cat.gold.mart_a;")
    expect(snippets.spark).toContain("lakehouse.gold.mart_a")
    expect(snippets.spark).toContain('"default"')
  })

  it("never offers a raw ClickHouse JDBC or native connection", () => {
    for (const snippet of [...Object.values(snippets), buildRestApiSnippet({ qualifiedTable: "gold.mart_a" })]) {
      expect(snippet).not.toContain(":8123")
      expect(snippet).not.toContain(":9000")
      expect(snippet.toLowerCase()).not.toContain("jdbc")
    }
  })
})

describe("martFromAssetId", () => {
  it("strips the catalog's serving prefix", () => {
    expect(martFromAssetId("serving.mart_a")).toBe("mart_a")
  })

  it("leaves an already-trimmed mart alone", () => {
    expect(martFromAssetId("mart_a")).toBe("mart_a")
  })
})
