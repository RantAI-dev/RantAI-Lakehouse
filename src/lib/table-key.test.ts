import { describe, expect, it } from "bun:test"

import { tableKeyOf } from "./table-key"

describe("tableKeyOf", () => {
  it("a silver row is its id, a bronze row is its table key, and a slug alone is no key", () => {
    expect(tableKeyOf({ id: "silver.orders" })).toBe("silver.orders")
    expect(tableKeyOf({ id: "raw-orders", tableKey: "bronze.raw_orders" })).toBe(
      "bronze.raw_orders"
    )
    expect(tableKeyOf({ id: "raw-orders", tableKey: null })).toBeNull()
  })
})
