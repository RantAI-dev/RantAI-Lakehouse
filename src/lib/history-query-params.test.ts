import { describe, expect, it } from "bun:test"
import { historyQueryString } from "./history-query-params"

describe("historyQueryString", () => {
  it("builds the query string in the contract's order from the fields the page set", () => {
    expect(
      historyQueryString({
        status: "failed",
        engine: "trino",
        minDurationMs: 1500,
        table: "bronze.orders",
        from: "2026-10-11T00:00:00Z",
        to: "2026-10-20T12:00:00.500Z",
        before: { at: "2026-10-20T11:59:59.999Z", id: "q-1729" },
      }),
    ).toBe(
      "status=failed&engine=trino&minDurationMs=1500&table=bronze.orders" +
        "&from=2026-10-11T00%3A00%3A00Z&to=2026-10-20T12%3A00%3A00.500Z" +
        "&before=2026-10-20T11%3A59%3A59.999Z%2Cq-1729",
    )
  })

  it("returns an empty string for no filter, an empty one, and one of empty strings", () => {
    expect(historyQueryString(undefined)).toBe("")
    expect(historyQueryString({})).toBe("")
    expect(historyQueryString({ table: "" })).toBe("")
  })

  it("keeps a zero minimum duration but drops empty text fields", () => {
    expect(historyQueryString({ minDurationMs: 0 })).toBe("minDurationMs=0")
  })

  it("percent-encodes the table text, so a literal % survives the trip", () => {
    expect(historyQueryString({ table: "ord%" })).toBe("table=ord%25")
    expect(historyQueryString({ table: "bronze orders" })).toBe("table=bronze+orders")
  })
})
