import { describe, expect, it } from "bun:test"
import type { LakehouseSnapshot } from "@/services/contracts/lakehouse"
import { pinSnapshot, snapshotRetentionText } from "./snapshot-picker"

const T = "SELECT * FROM icecat_api.`bronze.orders`"

describe("pinSnapshot", () => {
  it("adds a SETTINGS clause on its own line when there is none", () => {
    expect(pinSnapshot(T, "clickhouse", "123")).toBe(`${T}\nSETTINGS iceberg_snapshot_id = 123`)
  })

  it("appends to an existing SETTINGS clause", () => {
    expect(pinSnapshot(`${T}\nSETTINGS max_threads = 1`, "clickhouse", "123")).toBe(
      `${T}\nSETTINGS max_threads = 1, iceberg_snapshot_id = 123`
    )
  })

  it("replaces an existing iceberg_snapshot_id, once", () => {
    const out = pinSnapshot(
      `${T}\nSETTINGS iceberg_snapshot_id = 999, max_threads = 1`,
      "clickhouse",
      "123"
    )
    expect(out).toBe(`${T}\nSETTINGS max_threads = 1, iceberg_snapshot_id = 123`)
    expect(out?.match(/iceberg_snapshot_id/g)?.length).toBe(1)
  })

  it("pins again over its own output without stacking", () => {
    const once = pinSnapshot(T, "clickhouse", "1") as string
    expect(pinSnapshot(once, "clickhouse", "2")).toBe(`${T}\nSETTINGS iceberg_snapshot_id = 2`)
  })

  it("handles a trailing semicolon and trailing whitespace", () => {
    expect(pinSnapshot(`${T};  \n`, "clickhouse", "123")).toBe(
      `${T}\nSETTINGS iceberg_snapshot_id = 123`
    )
    expect(pinSnapshot(`${T}\nSETTINGS max_threads = 1;\n`, "clickhouse", "123")).toBe(
      `${T}\nSETTINGS max_threads = 1, iceberg_snapshot_id = 123`
    )
  })

  it("does not take the word SETTINGS in a string literal for the clause", () => {
    const sql = "SELECT 'SETTINGS x = 1' AS s FROM t"
    expect(pinSnapshot(sql, "clickhouse", "123")).toBe(`${sql}\nSETTINGS iceberg_snapshot_id = 123`)
  })

  it("does not take a SETTINGS clause inside a subquery or a comment for the outer one", () => {
    const sql = "SELECT * FROM (SELECT 1 SETTINGS max_threads = 1) x -- SETTINGS a = 1"
    expect(pinSnapshot(sql, "clickhouse", "123")).toBe(
      "SELECT * FROM (SELECT 1 SETTINGS max_threads = 1) x -- SETTINGS a = 1\nSETTINGS iceberg_snapshot_id = 123"
    )
  })

  it("drops a trailing comment from an existing clause rather than being swallowed by it", () => {
    expect(pinSnapshot(`${T}\nSETTINGS max_threads = 1 -- cap`, "clickhouse", "123")).toBe(
      `${T}\nSETTINGS max_threads = 1, iceberg_snapshot_id = 123`
    )
  })

  it("keeps a 19-digit id digit for digit", () => {
    const bigId = "7539123456789012345"
    expect(Number(bigId).toString()).not.toBe(bigId)
    expect(pinSnapshot(T, "clickhouse", bigId)).toEndWith(`iceberg_snapshot_id = ${bigId}`)
  })

  it("writes nothing for Trino or for an id that is not a plain number", () => {
    expect(pinSnapshot(T, "trino", "123")).toBeNull()
    expect(pinSnapshot(T, "clickhouse", "1; DROP TABLE x")).toBeNull()
  })
})

// `DATA-21` D5: on Trino a version belongs to ONE table, so the pin is
// written on that table's own reference.
describe("pinSnapshot on Trino", () => {
  const T = "SELECT * FROM iceberg.bronze.orders"

  it("writes the version after the table the caller picked", () => {
    expect(pinSnapshot(T, "trino", "123", "bronze.orders")).toBe(
      "SELECT * FROM iceberg.bronze.orders FOR VERSION AS OF 123"
    )
  })

  it("finds the table inside a catalog-qualified reference", () => {
    expect(pinSnapshot(T, "trino", "123", "bronze.orders")).toContain(
      "iceberg.bronze.orders FOR VERSION AS OF 123"
    )
  })

  it("does not match a longer table that shares the prefix", () => {
    expect(pinSnapshot("SELECT * FROM bronze.orders_archive", "trino", "123", "bronze.orders")).toBeNull()
  })

  it("replaces an earlier pin instead of stacking a second one", () => {
    const once = pinSnapshot(T, "trino", "1", "bronze.orders") as string
    expect(pinSnapshot(once, "trino", "2", "bronze.orders")).toBe(
      "SELECT * FROM iceberg.bronze.orders FOR VERSION AS OF 2"
    )
  })

  it("writes nothing when the statement does not read that table", () => {
    expect(pinSnapshot("SELECT * FROM bronze.invoices", "trino", "123", "bronze.orders")).toBeNull()
  })

  it("writes nothing inside a ClickHouse-style quoted table name", () => {
    expect(
      pinSnapshot("SELECT * FROM icecat_api.`bronze.orders`", "trino", "123", "bronze.orders")
    ).toBeNull()
  })

  it("writes the version after a double-quoted table reference", () => {
    expect(pinSnapshot('SELECT * FROM iceberg."bronze"."orders"', "trino", "123", "bronze.orders")).toBe(
      'SELECT * FROM iceberg."bronze"."orders" FOR VERSION AS OF 123'
    )
  })

  it("writes nothing without a table to pin", () => {
    expect(pinSnapshot(T, "trino", "123")).toBeNull()
  })

  it("writes nothing for an id that is not a plain number", () => {
    expect(pinSnapshot(T, "trino", "1; DROP TABLE x", "bronze.orders")).toBeNull()
  })

  it("keeps a 19-digit id digit for digit", () => {
    const bigId = "7539123456789012345"
    expect(pinSnapshot(T, "trino", bigId, "bronze.orders")).toContain(
      `FOR VERSION AS OF ${bigId}`
    )
  })
})

function snap(id: string, timestampMs: number): LakehouseSnapshot {
  return {
    id,
    parentId: null,
    timestampMs,
    operation: "append",
    author: null,
    summary: { addedRecords: null, deletedRecords: null, totalRecords: null, totalDataFiles: null },
  }
}

describe("snapshotRetentionText", () => {
  it("says so when there are no versions", () => {
    expect(snapshotRetentionText([])).toBe("No versions yet.")
  })

  it("names the single version's date", () => {
    expect(snapshotRetentionText([snap("1", 1_767_225_600_000)])).toMatch(/^1 version, from \S/)
  })

  it("counts the versions and dates the oldest, whatever the list order", () => {
    const text = snapshotRetentionText([
      snap("4", 4_000_000_000_000),
      snap("1", 1_000_000_000_000),
      snap("2", 2_000_000_000_000),
      snap("3", 3_000_000_000_000),
    ])
    expect(text).toMatch(/^4 versions, the oldest from \S/)
    expect(text).toContain("2001")
  })
})

