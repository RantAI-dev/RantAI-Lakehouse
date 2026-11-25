import { describe, expect, it } from "bun:test"
import { availableQueryEngines } from "./query-engines"

describe("availableQueryEngines (BI-44)", () => {
  it("keeps both engines in full mode", () => {
    expect(availableQueryEngines("full", "trino")).toEqual(["clickhouse", "trino"])
    expect(availableQueryEngines("full", "clickhouse")).toEqual(["clickhouse", "trino"])
  })

  it("keeps both engines when the mode is absent or still loading", () => {
    expect(availableQueryEngines(undefined, undefined)).toEqual(["clickhouse", "trino"])
    expect(availableQueryEngines("", "trino")).toEqual(["clickhouse", "trino"])
    expect(availableQueryEngines("something-else", "trino")).toEqual(["clickhouse", "trino"])
  })

  it("offers only the configured engine in bi-only mode", () => {
    expect(availableQueryEngines("bi-only", "trino")).toEqual(["trino"])
    expect(availableQueryEngines("bi-only", "clickhouse")).toEqual(["clickhouse"])
  })

  it("falls back to the API's ClickHouse default for an unstated bi-only engine", () => {
    expect(availableQueryEngines("bi-only")).toEqual(["clickhouse"])
    expect(availableQueryEngines("bi-only", "")).toEqual(["clickhouse"])
    expect(availableQueryEngines("bi-only", "other-engine")).toEqual(["clickhouse"])
  })
})
