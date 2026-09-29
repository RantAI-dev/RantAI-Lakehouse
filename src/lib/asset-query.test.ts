import { describe, expect, it } from "bun:test"
import { assetQueryStudioHref, assetQueryTarget, assetStarterSql } from "./asset-query"

type Target = Parameters<typeof assetStarterSql>[0]

const iceberg: Target = {
  layer: "raw",
  type: "iceberg-table",
  tableName: "demo_orders_param",
  namespace: "primer",
  name: "Demo Orders Param",
  schema: [
    { name: "id", dataType: "long" },
    { name: "created at", dataType: "timestamptz" },
  ],
}

const plain: Target = { ...iceberg, layer: "gold", type: "table", tableName: null }

describe("assetQueryTarget", () => {
  it("addresses an Iceberg candidate through Trino's iceberg catalog", () => {
    expect(assetQueryTarget(iceberg)).toEqual({
      engine: "trino",
      table: "iceberg.bronze.demo_orders_param",
    })
  })

  it("quotes a namespace.name that is not a simple identifier", () => {
    expect(assetQueryTarget(plain)).toEqual({
      engine: "clickhouse",
      table: 'primer."Demo Orders Param"',
    })
  })
})

describe("assetQueryTarget with an API-resolved target", () => {
  it("uses the table the API resolved over any guess", () => {
    const target = { engine: "clickhouse", table: "icecat_api.`bronze.demo_orders_param`" } as const
    expect(assetQueryTarget({ ...iceberg, queryTarget: target })).toEqual(target)
    expect(assetStarterSql({ ...iceberg, queryTarget: target })).toContain(
      "FROM icecat_api.`bronze.demo_orders_param`"
    )
  })
})

describe("assetStarterSql", () => {
  it("lists columns, quoting the ones that need it", () => {
    expect(assetStarterSql(iceberg)).toBe(
      'SELECT\n  id,\n  "created at"\nFROM iceberg.bronze.demo_orders_param\nLIMIT 100'
    )
  })

  it("falls back to * with no schema", () => {
    expect(assetStarterSql({ ...plain, schema: [] })).toContain("SELECT\n  *\n")
  })
})

describe("assetQueryStudioHref", () => {
  it("carries both the SQL and the engine it was written for", () => {
    const url = new URL(assetQueryStudioHref(iceberg), "http://x")
    expect(url.pathname).toBe("/query-studio")
    expect(url.searchParams.get("engine")).toBe("trino")
    expect(url.searchParams.get("sql")).toBe(assetStarterSql(iceberg))
  })
})
