import { describe, expect, it } from "bun:test"
import {
  assetHasReadableTable,
  assetQueryStudioHref,
  assetQueryTarget,
  assetSnapshotQueryHref,
  assetStarterSql,
  assetStudioTarget,
  listedBronzeTable,
  NO_READABLE_TABLE,
  trinoBronzeName,
} from "./asset-query"
import type { LakehouseTableDetail } from "@/services/contracts/lakehouse"

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

/**
 * The Iceberg table, loaded: the detail the page fetched. Its presence is
 * what proves the table exists — the registry's `tableName` alone does not,
 * and a candidate whose table is not there answers "table does not exist"
 * on Trino (owner, 2026-11-08).
 */
const table: LakehouseTableDetail = {
  schema: [],
  partitionSpec: [],
  properties: {},
  snapshots: [],
  stats: {
    fileCount: 1,
    smallFileCount: null,
    smallFileThresholdBytes: null,
    recordCount: 2000,
    totalBytes: 40287,
    snapshotCount: 1,
    metadataLogCount: 0,
  },
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
    expect(assetStarterSql({ ...plain, queryTarget: target }, null)).toContain(
      "FROM icecat_api.`bronze.demo_orders_param`"
    )
  })
})

describe("assetStarterSql", () => {
  it("lists columns, quoting the ones that need it", () => {
    expect(assetStarterSql(iceberg, table)).toBe(
      'SELECT\n  id,\n  "created at"\nFROM iceberg.bronze.demo_orders_param\nLIMIT 100'
    )
  })

  it("falls back to * with no schema", () => {
    expect(assetStarterSql({ ...plain, schema: [] }, null)).toContain("SELECT\n  *\n")
  })
})

describe("assetQueryStudioHref", () => {
  it("carries both the SQL and the engine it was written for", () => {
    const url = new URL(assetQueryStudioHref(iceberg, table), "http://x")
    expect(url.pathname).toBe("/query-studio")
    expect(url.searchParams.get("engine")).toBe("trino")
    expect(url.searchParams.get("sql")).toBe(assetStarterSql(iceberg, table))
  })
})

describe("assetSnapshotQueryHref", () => {
  const sqlOf = (href: string | null) => new URL(href ?? "", "http://x").searchParams.get("sql")

  // `DATA-16` F6 had the API refuse `FOR VERSION AS OF` on Trino, so a
  // Trino asset got no link. `DATA-21` made the API read it, and the owner
  // decided on 2026-11-06 that these links open a Trino tab.
  it("pins a Trino read of the Iceberg table", () => {
    expect(sqlOf(assetSnapshotQueryHref(iceberg, "42", table))).toContain(
      "FROM iceberg.bronze.demo_orders_param FOR VERSION AS OF 42"
    )
  })

  it("offers nothing when the asset is not Iceberg", () => {
    expect(assetSnapshotQueryHref(plain, "42", null)).toBeNull()
  })

  it("refuses an id that is not a plain number", () => {
    expect(assetSnapshotQueryHref(iceberg, "42; DROP TABLE x", table)).toBeNull()
  })
})

describe("assetStudioTarget", () => {
  it("opens a Bronze table Trino reads on a Trino tab, whatever name the API sent", () => {
    expect(assetStudioTarget(readable, table)).toEqual({
      engine: "trino",
      table: "iceberg.bronze.demo_orders_param",
    })
  })

  it("quotes a table name that is not a plain identifier", () => {
    const odd = { ...readable, tableName: "order lines" }
    expect(assetStudioTarget(odd, table).table).toBe('iceberg.bronze."order lines"')
  })

  it("keeps the API's ClickHouse name for a Silver stand-in", () => {
    expect(assetStudioTarget(standIn, table)).toEqual(standIn.queryTarget as never)
  })

  it("keeps the ClickHouse name of an asset that is not a Bronze table", () => {
    expect(assetStudioTarget(plain, null)).toEqual({ engine: "clickhouse", table: 'primer."Demo Orders Param"' })
  })

  it("leaves assetQueryTarget on the API's ClickHouse name", () => {
    expect(assetQueryTarget(readable)).toEqual(dataLake)
  })
})

describe("a catalog link to a Bronze table the API can read", () => {
  it("starts the query on iceberg.bronze, not on the DataLakeCatalog name", () => {
    expect(assetStarterSql(readable, table)).toBe(
      'SELECT\n  id,\n  "created at"\nFROM iceberg.bronze.demo_orders_param\nLIMIT 100'
    )
  })

  it("opens Query Studio on a Trino tab", () => {
    const params = queryOf(assetQueryStudioHref(readable, table))
    expect(params.get("engine")).toBe("trino")
    expect(params.get("sql")).toContain("FROM iceberg.bronze.demo_orders_param\n")
  })

  it("quotes an odd table name in the link", () => {
    const params = queryOf(assetQueryStudioHref({ ...readable, tableName: "order lines" }, table))
    expect(params.get("engine")).toBe("trino")
    expect(params.get("sql")).toContain('FROM iceberg.bronze."order lines"\n')
  })

  it("pins a snapshot on Trino, on the table's own reference", () => {
    const href = assetSnapshotQueryHref(readable, "248842615326512766", table)
    const params = queryOf(href)
    expect(params.get("engine")).toBe("trino")
    expect(params.get("sql")).toBe(
      'SELECT\n  id,\n  "created at"\nFROM iceberg.bronze.demo_orders_param FOR VERSION AS OF 248842615326512766\nLIMIT 100'
    )
    expect(params.get("sql")).not.toContain("iceberg_snapshot_id")
  })

  it("pins a snapshot of a table with an odd name on that table", () => {
    const params = queryOf(assetSnapshotQueryHref({ ...readable, tableName: "order lines" }, "7", table))
    expect(params.get("sql")).toContain('FROM iceberg.bronze."order lines" FOR VERSION AS OF 7\n')
  })
})

describe("a catalog link to a Silver stand-in or a non-Bronze table", () => {
  it("keeps the stand-in's ClickHouse link", () => {
    const params = queryOf(assetQueryStudioHref(standIn, table))
    expect(params.get("engine")).toBe("clickhouse")
    expect(params.get("sql")).toContain("FROM silver.`demo_orders_param`")
  })

  it("keeps the ClickHouse link of a table that is not Bronze", () => {
    const params = queryOf(assetQueryStudioHref(plain, null))
    expect(params.get("engine")).toBe("clickhouse")
    expect(params.get("sql")).toContain('FROM primer."Demo Orders Param"')
  })

  it("offers no snapshot link for a stand-in, which has no snapshots of its own", () => {
    expect(assetSnapshotQueryHref(standIn, "42", table)).toBeNull()
  })
})

// The table is not there: the registry named one, but the detail the page
// loads is `null` — while it loads, after it failed, after a `not_found`.
// All three read as `null` here, so the link is the ClickHouse one the
// catalog had before #409 (owner, 2026-11-08).
describe("a catalog link to a Bronze table whose Iceberg table is not there", () => {
  it("opens the ClickHouse name the API sent, not Trino", () => {
    expect(assetStudioTarget(readable, null)).toEqual(dataLake)
  })

  it("opens the asset's own namespace.name when the API named no table", () => {
    expect(assetStudioTarget(iceberg, null)).toEqual({
      engine: "clickhouse",
      table: 'primer."Demo Orders Param"',
    })
  })

  it("starts the query on the ClickHouse name", () => {
    expect(assetStarterSql(readable, null)).toContain("FROM icecat_api.`bronze.demo_orders_param`")
  })

  it("opens Query Studio on a ClickHouse tab", () => {
    const params = queryOf(assetQueryStudioHref(readable, null))
    expect(params.get("engine")).toBe("clickhouse")
    expect(params.get("sql")).toContain("FROM icecat_api.`bronze.demo_orders_param`")
  })

  it("offers no snapshot link, for a table with no versions of its own", () => {
    expect(assetSnapshotQueryHref(readable, "42", null)).toBeNull()
  })
})

// The rule of 2026-11-09: an asset with no readable table offers no Query
// Studio link. It shows one sentence instead.
describe("assetHasReadableTable", () => {
  it("gives the table the API resolved", () => {
    expect(assetHasReadableTable(readable, null, false)).toBe(true)
  })

  it("gives the Silver model that stands in for a Bronze table", () => {
    expect(assetHasReadableTable(standIn, null, false)).toBe(true)
  })

  it("gives the Iceberg table that is there", () => {
    expect(assetHasReadableTable(iceberg, table, false)).toBe(true)
  })

  it("takes the table away from an asset that has none", () => {
    expect(assetHasReadableTable(iceberg, null, false)).toBe(false)
  })

  it("waits for the read that is still running", () => {
    // The load has not finished, so the sentence must not show yet.
    expect(assetHasReadableTable(iceberg, null, true)).toBe(true)
  })
})

describe("NO_READABLE_TABLE", () => {
  it("is the sentence the console shows for an asset with no readable table", () => {
    expect(NO_READABLE_TABLE).toBe("No readable table for this asset yet.")
  })
})

// QA LH-QS-4-08 (QS-7), decision 11: the Bronze list row already carries the
// registry table name (`tableKey`), so Query Studio knows a Bronze table's
// Trino name before the asset's detail is opened. It says the table might
// exist; a read of the `bronze` namespace's table list proves it.
describe("listedBronzeTable", () => {
  it("reads the table name of a Bronze list row and nothing from any other row", () => {
    // The Bronze row `bronze_catalog_row` builds.
    expect(
      listedBronzeTable({ layer: "raw", type: "iceberg-table", tableKey: "bronze.orders" })
    ).toBe("orders")
    // A Bronze asset whose type says Iceberg too, as the detail route files it.
    expect(
      listedBronzeTable({ layer: "bronze", type: "table", tableKey: "bronze.order_lines" })
    ).toBe("order_lines")
    // A Silver row carries no `tableKey` at all.
    expect(listedBronzeTable({ layer: "silver", type: "table" })).toBeNull()
    // A row whose key is explicitly null names no table.
    expect(listedBronzeTable({ layer: "raw", type: "iceberg-table", tableKey: null })).toBeNull()
    // A key in another namespace is not a Bronze table, whatever the layer.
    expect(
      listedBronzeTable({ layer: "raw", type: "iceberg-table", tableKey: "silver.orders" })
    ).toBeNull()
    // A row that is not an Iceberg candidate keeps no name.
    expect(
      listedBronzeTable({ layer: "gold", type: "table", tableKey: "bronze.orders" })
    ).toBeNull()
    // An empty name after the dot is no name.
    expect(
      listedBronzeTable({ layer: "raw", type: "iceberg-table", tableKey: "bronze." })
    ).toBeNull()
  })
})
