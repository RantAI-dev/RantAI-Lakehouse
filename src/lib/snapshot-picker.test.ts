import { describe, expect, it } from "bun:test"
import type { LakehouseSnapshot } from "@/services/contracts/lakehouse"
import { insertAsOfClause, pinSnapshot, snapshotRetentionText } from "./snapshot-picker"

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

function snap(id: string, timestampMs: number): LakehouseSnapshot {
  return {
    id,
    parentId: null,
    timestampMs,
    operation: "append",
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


describe("insertAsOfClause", () => {
  it("inserts FOR VERSION AS OF after the table reference", () => {
    const sql = "SELECT * FROM bronze.orders"
    const result = insertAsOfClause(sql, "bronze.orders", "123")
    expect(result).toBe("SELECT * FROM bronze.orders FOR VERSION AS OF 123")
  })

  it("keeps a large snapshot id's digits unchanged, never rounding through Number", () => {
    // 7539123456789012345 exceeds Number.MAX_SAFE_INTEGER (9007199254740991
    // is actually larger — pick a value whose round-trip through Number()
    // demonstrably mangles it) — this id, run through Number(), loses its
    // trailing digits to floating-point rounding.
    const bigId = "7539123456789012345"
    expect(Number(bigId).toString()).not.toBe(bigId)
    const result = insertAsOfClause("SELECT * FROM bronze.orders", "bronze.orders", bigId)
    expect(result).toBe(`SELECT * FROM bronze.orders FOR VERSION AS OF ${bigId}`)
  })

  it("matches a namespace.table reference even when catalog-qualified in the SQL", () => {
    const result = insertAsOfClause(
      "SELECT * FROM iceberg.bronze.orders",
      "bronze.orders",
      "7539123456789012345"
    )
    expect(result).toBe(
      "SELECT * FROM iceberg.bronze.orders FOR VERSION AS OF 7539123456789012345"
    )
  })

  it("leaves sql unchanged when the table reference is not present", () => {
    const sql = "SELECT 1"
    expect(insertAsOfClause(sql, "bronze.orders", "123")).toBe(sql)
  })

  it("never matches a longer table name sharing the same prefix", () => {
    const sql = "SELECT * FROM bronze.orders_archive"
    expect(insertAsOfClause(sql, "bronze.orders", "123")).toBe(sql)
  })

  it("leaves the statement unchanged when a FOR VERSION AS OF clause already follows", () => {
    const sql = "SELECT * FROM bronze.orders FOR VERSION AS OF 999"
    expect(insertAsOfClause(sql, "bronze.orders", "123")).toBe(sql)
  })

  it("leaves the statement unchanged when a FOR TIMESTAMP AS OF clause already follows", () => {
    const sql = "SELECT * FROM bronze.orders FOR TIMESTAMP AS OF TIMESTAMP '2026-01-01 00:00:00 UTC'"
    expect(insertAsOfClause(sql, "bronze.orders", "123")).toBe(sql)
  })

  it("leaves a match inside a single-quoted string literal unchanged", () => {
    const sql = "SELECT 'bronze.orders' AS label"
    expect(insertAsOfClause(sql, "bronze.orders", "123")).toBe(sql)
  })
})
