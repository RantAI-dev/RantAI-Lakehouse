import { describe, expect, it } from "bun:test"
import { insertAsOfClause } from "./snapshot-picker"

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
