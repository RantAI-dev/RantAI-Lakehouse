import { describe, expect, test } from "bun:test"
import { readFileSync } from "node:fs"
import { join } from "node:path"
import { SAAS_VENDORS, saasAccountProblem, saasVendorById, saasVendorForType } from "./saas-vendors"

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

// QA LH-SRC-3-11 (SRC-41 D13): the console's copy of `SaasVendor::validate_account`.
// The examples are the Rust tests' (`saas_vendors.rs`), so the two copies are
// held to the same cases.
const vendor = (id: string) => {
  const found = saasVendorById(id)
  if (!found) throw new Error(`no vendor ${id}`)
  return found
}

describe("the account each vendor's form takes (SRC-41 D13)", () => {
  test("accepts the account each vendor's own form takes", () => {
    for (const id of ["salesforce", "servicenow", "zendesk", "confluence"]) {
      expect(saasAccountProblem(vendor(id), "acme")).toBeNull()
    }
    for (const id of ["ga4", "hubspot"]) {
      expect(saasAccountProblem(vendor(id), "123456789")).toBeNull()
    }
    expect(saasAccountProblem(vendor("netsuite"), "1234567_SB1")).toBeNull()
    expect(saasAccountProblem(vendor("workday"), "wd2-impl-services1.workday.com/acme_tenant")).toBeNull()
    expect(saasAccountProblem(vendor("workday"), "a.b.c.myworkday.com/t_1")).toBeNull()
    expect(saasAccountProblem(vendor("sharepoint"), "acme/00000000-0000-0000-0000-000000000001")).toBeNull()
    expect(
      saasAccountProblem(vendor("dynamics365"), "acme.crm4.dynamics.com/contoso.example.invalid")
    ).toBeNull()
    expect(
      saasAccountProblem(vendor("dynamics365"), "Acme.CRM.dynamics.com/00000000-0000-0000-0000-000000000001")
    ).toBeNull()
  })

  test("refuses what no vendor's form takes", () => {
    const refused = [
      "evil.example.invalid",
      "https://acme",
      "acme@evil",
      "user:pass@acme",
      "acme:8443",
      "acme/path",
      "acme.evil",
      " acme",
      "acme ",
      "",
      "-acme",
      "acme-",
      "ac me",
      "127.0.0.1",
    ]
    for (const vendor_ of SAAS_VENDORS) {
      for (const account of refused) {
        expect(saasAccountProblem(vendor_, account)).toBe(`This is not in the form ${vendor_.label} uses.`)
      }
    }
  })

  test("refuses each vendor's near misses", () => {
    const workday = vendor("workday")
    for (const account of [
      "wd2.workday.com",
      "wd2.workday.com/a/b",
      "a.b.c.d.workday.com/t",
      "wd2.workday.com.evil.invalid/t",
      "wd2.workday.org/t",
    ]) {
      expect(saasAccountProblem(workday, account)).not.toBeNull()
    }

    const dynamics = vendor("dynamics365")
    const tenant = "/00000000-0000-0000-0000-000000000001"
    for (const host of [
      "acme.dynamics.com",
      "acme.crm123.dynamics.com",
      "acme.crmx.dynamics.com",
      "a.b.crm.dynamics.com",
    ]) {
      expect(saasAccountProblem(dynamics, `${host}${tenant}`)).not.toBeNull()
    }
    expect(saasAccountProblem(dynamics, `127.0.0.1${tenant}`)).not.toBeNull()

    const netsuite = vendor("netsuite")
    for (const account of ["1234567_", "_1234567"]) {
      expect(saasAccountProblem(netsuite, account)).not.toBeNull()
    }

    const badTenants = [
      "common",
      "contoso",
      "1.2.3.4",
      "00000000-0000-0000-0000-00000000000g",
    ]
    for (const bad of badTenants) {
      expect(saasAccountProblem(vendor("sharepoint"), `acme/${bad}`)).not.toBeNull()
      expect(saasAccountProblem(dynamics, `acme.crm4.dynamics.com/${bad}`)).not.toBeNull()
    }
    // No tenant at all.
    expect(saasAccountProblem(vendor("sharepoint"), "acme")).not.toBeNull()
    expect(saasAccountProblem(vendor("sharepoint"), "acme/")).not.toBeNull()
  })
})
