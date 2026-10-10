// A policy added from the Access tab must be one the engine enforces: it
// binds to the table the asset's rows are read from, names at least one
// role, and does something — masks a column or filters rows.
import { describe, expect, it } from "bun:test"
import type { AssetDetail } from "@/services/contracts/assets"
import { policyCondition, policyTable } from "./asset-access"

const asset = (a: Partial<AssetDetail>) => a as AssetDetail

describe("policyTable", () => {
  it("is the table the API says a policy binds to", () => {
    expect(
      policyTable(
        asset({
          id: "demo-orders",
          tableKey: "bronze.demo_orders",
          queryTarget: { engine: "clickhouse", table: "silver.demo_orders", policyTable: "silver.demo_orders" },
        })
      )
    ).toBe("silver.demo_orders")
  })

  it("is worked out from the query target for an API build that does not say", () => {
    expect(
      policyTable(asset({ id: "demo-orders", queryTarget: { engine: "clickhouse", table: "icecat_api.`bronze.demo_orders`" } }))
    ).toBe("bronze.demo_orders")
  })

  it("falls back to the table key when nothing readable backs the asset", () => {
    expect(policyTable(asset({ id: "serving.mart_orders", tableKey: "serving.mart_orders" }))).toBe("serving.mart_orders")
  })
})

describe("policyCondition", () => {
  it("needs a role and something to enforce", () => {
    expect(policyCondition("bronze.t", { roles: " , ", mask: ["email"], rowFilter: "" })).toBeNull()
    expect(policyCondition("bronze.t", { roles: "Analyst", mask: [], rowFilter: "  " })).toBeNull()
  })

  it("carries a row filter only when one is written", () => {
    expect(policyCondition("bronze.t", { roles: "Analyst, Viewer ", mask: ["email"], rowFilter: "" })).toEqual({
      roles: ["Analyst", "Viewer"],
      table: "bronze.t",
      mask: ["email"],
    })
    expect(policyCondition("bronze.t", { roles: "Analyst", mask: [], rowFilter: " region = 'ID' " })).toEqual({
      roles: ["Analyst"],
      table: "bronze.t",
      mask: [],
      rowFilter: "region = 'ID'",
    })
  })
})
