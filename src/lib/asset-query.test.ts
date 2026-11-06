import { describe, expect, it } from "bun:test"
import {
  assetQueryStudioHref,
  assetQueryTarget,
  assetSnapshotQueryHref,
  assetStarterSql,
  assetStudioTarget,
  trinoBronzeName,
} from "./asset-query"

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

/** What the API sends for a Bronze table it can read: the ClickHouse name. */
const dataLake = {
  engine: "clickhouse",
  table: "icecat_api.`bronze.demo_orders_param`",
  policyTable: "bronze.demo_orders_param",
} as const

/** A Bronze table the API can read, as the catalog loads it. */
const readable: Target & { tableKey: string } = {
  ...iceberg,
  tableKey: "bronze.demo_orders_param",
  queryTarget: dataLake,
}

/** The Silver model standing in for a Bronze table that cannot be read. */
const standIn: Target & { tableKey: string } = {
  ...readable,
  queryTarget: { engine: "clickhouse", table: "silver.`demo_orders_param`", policyTable: "silver.demo_orders_param" },
}

const queryOf = (href: string | null) => new URL(href ?? "", "http://x").searchParams

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

describe("trinoBronzeName", () => {
  it("is the iceberg catalog's bronze schema and the table", () => {
    expect(trinoBronzeName("orders")).toBe("iceberg.bronze.orders")
  })

  it("quotes a table name that is not a plain identifier", () => {
    expect(trinoBronzeName("order lines")).toBe('iceberg.bronze."order lines"')
  })
})

describe("assetQueryTarget with an API-resolved target", () => {
  it("uses the table the API resolved over any guess", () => {
    const target = { engine: "clickhouse", table: "icecat_api.`bronze.demo_orders_param`" } as const
    expect(assetQueryTarget({ ...iceberg, queryTarget: target })).toEqual(target)
    // The starter query reads it for a row that is not opened on Trino: a
    // Bronze table opened on Trino is named by `assetStudioTarget` instead.
    expect(assetStarterSql({ ...plain, queryTarget: target })).toContain(
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

describe("assetSnapshotQueryHref", () => {
  const sqlOf = (href: string | null) => new URL(href ?? "", "http://x").searchParams.get("sql")

  // `DATA-16` F6 had the API refuse `FOR VERSION AS OF` on Trino, so a
  // Trino asset got no link. `DATA-21` made the API read it, and the owner
  // decided on 2026-11-06 that these links open a Trino tab.
  it("pins a Trino read of the Iceberg table", () => {
    expect(sqlOf(assetSnapshotQueryHref(iceberg, "42"))).toContain(
      "FROM iceberg.bronze.demo_orders_param FOR VERSION AS OF 42"
    )
  })

  it("offers nothing when the asset is not Iceberg", () => {
    expect(assetSnapshotQueryHref(plain, "42")).toBeNull()
  })

  it("refuses an id that is not a plain number", () => {
    expect(assetSnapshotQueryHref(iceberg, "42; DROP TABLE x")).toBeNull()
  })
})

describe("assetStudioTarget", () => {
  it("opens a Bronze table Trino reads on a Trino tab, whatever name the API sent", () => {
    expect(assetStudioTarget(readable)).toEqual({
      engine: "trino",
      table: "iceberg.bronze.demo_orders_param",
    })
  })

  it("quotes a table name that is not a plain identifier", () => {
    const odd = { ...readable, tableName: "order lines" }
    expect(assetStudioTarget(odd).table).toBe('iceberg.bronze."order lines"')
  })

  it("keeps the API's ClickHouse name for a Silver stand-in", () => {
    expect(assetStudioTarget(standIn)).toEqual(standIn.queryTarget as never)
  })

  it("keeps the ClickHouse name of an asset that is not a Bronze table", () => {
    expect(assetStudioTarget(plain)).toEqual({ engine: "clickhouse", table: 'primer."Demo Orders Param"' })
  })

  it("leaves assetQueryTarget on the API's ClickHouse name", () => {
    expect(assetQueryTarget(readable)).toEqual(dataLake)
  })
})

describe("a catalog link to a Bronze table the API can read", () => {
  it("starts the query on iceberg.bronze, not on the DataLakeCatalog name", () => {
    expect(assetStarterSql(readable)).toBe(
      'SELECT\n  id,\n  "created at"\nFROM iceberg.bronze.demo_orders_param\nLIMIT 100'
    )
  })

  it("opens Query Studio on a Trino tab", () => {
    const params = queryOf(assetQueryStudioHref(readable))
    expect(params.get("engine")).toBe("trino")
    expect(params.get("sql")).toContain("FROM iceberg.bronze.demo_orders_param\n")
  })

  it("quotes an odd table name in the link", () => {
    const params = queryOf(assetQueryStudioHref({ ...readable, tableName: "order lines" }))
    expect(params.get("engine")).toBe("trino")
    expect(params.get("sql")).toContain('FROM iceberg.bronze."order lines"\n')
  })

  it("pins a snapshot on Trino, on the table's own reference", () => {
    const href = assetSnapshotQueryHref(readable, "248842615326512766")
    const params = queryOf(href)
    expect(params.get("engine")).toBe("trino")
    expect(params.get("sql")).toBe(
      'SELECT\n  id,\n  "created at"\nFROM iceberg.bronze.demo_orders_param FOR VERSION AS OF 248842615326512766\nLIMIT 100'
    )
    expect(params.get("sql")).not.toContain("iceberg_snapshot_id")
  })

  it("pins a snapshot of a table with an odd name on that table", () => {
    const params = queryOf(assetSnapshotQueryHref({ ...readable, tableName: "order lines" }, "7"))
    expect(params.get("sql")).toContain('FROM iceberg.bronze."order lines" FOR VERSION AS OF 7\n')
  })
})

describe("a catalog link to a Silver stand-in or a non-Bronze table", () => {
  it("keeps the stand-in's ClickHouse link", () => {
    const params = queryOf(assetQueryStudioHref(standIn))
    expect(params.get("engine")).toBe("clickhouse")
    expect(params.get("sql")).toContain("FROM silver.`demo_orders_param`")
  })

  it("keeps the ClickHouse link of a table that is not Bronze", () => {
    const params = queryOf(assetQueryStudioHref(plain))
    expect(params.get("engine")).toBe("clickhouse")
    expect(params.get("sql")).toContain('FROM primer."Demo Orders Param"')
  })

  it("offers no snapshot link for a stand-in, which has no snapshots of its own", () => {
    expect(assetSnapshotQueryHref(standIn, "42")).toBeNull()
  })
})
