import { describe, expect, it } from "bun:test"
import {
  EMPTY_HISTORY_ADDRESS,
  historyAddress,
  historyQueryString,
  readHistoryAddress,
} from "./history-query-params"

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
        page: 3,
      }),
    ).toBe(
      "status=failed&engine=trino&minDurationMs=1500&table=bronze.orders" +
        "&from=2026-10-11T00%3A00%3A00Z&to=2026-10-20T12%3A00%3A00.500Z" +
        "&page=3",
    )
  })

  it("returns an empty string for no filter, an empty one, and one of empty strings", () => {
    expect(historyQueryString(undefined)).toBe("")
    expect(historyQueryString({})).toBe("")
    expect(historyQueryString({ table: "" })).toBe("")
  })

  it("leaves the page out for the first page and writes it from the second", () => {
    expect(historyQueryString({ status: "failed" })).toBe("status=failed")
    expect(historyQueryString({ page: 2 })).toBe("page=2")
  })

  it("keeps a zero minimum duration but drops empty text fields", () => {
    expect(historyQueryString({ minDurationMs: 0 })).toBe("minDurationMs=0")
  })

  it("percent-encodes the table text, so a literal % survives the trip", () => {
    expect(historyQueryString({ table: "ord%" })).toBe("table=ord%25")
    expect(historyQueryString({ table: "bronze orders" })).toBe("table=bronze+orders")
  })

  it("writes the run id when one is asked for", () => {
    expect(historyQueryString({ id: "q-1" })).toBe("id=q-1")
  })
})

// QS-12 (LH-QS-4-06), D17: the page's own address, read on entry and written
// back with replaceState. Unknown values are dropped, not sent to the server.
describe("history address", () => {
  const full = {
    status: "failed",
    engine: "trino",
    minDuration: "1500",
    table: "bronze.orders",
    from: "2026-10-11",
    to: "2026-10-20",
    page: 3,
  }

  it("round trips every field", () => {
    const search = historyAddress(full)
    expect(search).toBe(
      "status=failed&engine=trino&minDurationMs=1500&table=bronze.orders" +
        "&from=2026-10-11&to=2026-10-20&page=3",
    )
    expect(readHistoryAddress(`?${search}`)).toEqual(full)
  })

  it("ignores a value the page does not know", () => {
    expect(readHistoryAddress("?status=nope&from=2026-1-1&page=0")).toEqual(
      EMPTY_HISTORY_ADDRESS,
    )
    expect(readHistoryAddress("?engine=presto&minDurationMs=soon&page=abc")).toEqual(
      EMPTY_HISTORY_ADDRESS,
    )
  })

  it("writes nothing for an untouched page and leaves the first page out", () => {
    expect(historyAddress(EMPTY_HISTORY_ADDRESS)).toBe("")
    expect(historyAddress({ ...full, page: 1 })).not.toContain("page=")
  })
})
