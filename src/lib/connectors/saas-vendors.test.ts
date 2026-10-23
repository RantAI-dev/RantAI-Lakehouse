import { describe, expect, test } from "bun:test"
import { readFileSync } from "node:fs"
import { join } from "node:path"
import { SAAS_VENDORS, saasVendorById, saasVendorForType } from "./saas-vendors"

describe("the console's vendor table", () => {
  test("equals the shared fixture the Rust and Python tables are compared with", () => {
    const fixture = JSON.parse(readFileSync(join(import.meta.dir, "../../../ops/fixtures/saas_vendors.json"), "utf8"))
    expect(JSON.parse(JSON.stringify(SAAS_VENDORS))).toEqual(fixture.vendors)
  })

  test("maps each of the ten connector types to its vendor, and an id to the same vendor", () => {
    expect(SAAS_VENDORS).toHaveLength(10)
    for (const vendor of SAAS_VENDORS) {
      expect(saasVendorForType(vendor.connectorType)).toBe(vendor)
      expect(saasVendorById(vendor.id)).toBe(vendor)
    }
    expect(saasVendorForType("PostgreSQL")).toBeUndefined()
    expect(saasVendorById(7)).toBeUndefined()
  })

  test("Salesforce offers no integration credential (decision 4)", () => {
    expect(saasVendorById("salesforce")?.integration).toBeNull()
  })
})
