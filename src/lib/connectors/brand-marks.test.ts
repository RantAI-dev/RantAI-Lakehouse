import { describe, expect, it } from "bun:test"
import { readFileSync } from "node:fs"
import { BRAND_MARKS, brandMarkForTypeName } from "@/lib/connectors/brand-marks"

describe("brandMarkForTypeName", () => {
  it("gives each of the eight products its own mark", () => {
    const expected: Record<string, string> = {
      PostgreSQL: "PostgreSQL",
      MySQL: "MySQL",
      MariaDB: "MariaDB",
      MongoDB: "MongoDB",
      Kafka: "Apache Kafka",
      "Google Sheets": "Google Sheets",
      "SAP / ERP": "SAP",
      MQTT: "MQTT",
    }
    for (const [name, title] of Object.entries(expected)) {
      expect(brandMarkForTypeName(name)?.title).toBe(title)
    }
    expect(new Set(Object.keys(expected).map((n) => brandMarkForTypeName(n)?.paths.join())).size).toBe(8)
  })

  it("gives each business-app type whose product Simple Icons publishes today its own mark (SRC-3 T7)", () => {
    const expected: Record<string, string> = {
      Confluence: "Confluence",
      "Google Analytics 4": "Google Analytics",
      HubSpot: "HubSpot",
      Zendesk: "Zendesk",
    }
    for (const [name, title] of Object.entries(expected)) {
      expect(brandMarkForTypeName(name)?.title).toBe(title)
    }
  })

  it("gives no mark to the business-app types Simple Icons does not publish a file for (Salesforce, ServiceNow, Workday Reports, Dynamics 365, NetSuite, SharePoint)", () => {
    for (const name of ["Salesforce", "ServiceNow", "Workday Reports", "Dynamics 365", "NetSuite", "SharePoint"]) {
      expect(brandMarkForTypeName(name)).toBeNull()
    }
  })

  it("gives a CDC type its database's mark", () => {
    expect(brandMarkForTypeName("PostgreSQL CDC")).toBe(brandMarkForTypeName("PostgreSQL"))
    expect(brandMarkForTypeName("MySQL CDC")).toBe(brandMarkForTypeName("MySQL"))
    expect(brandMarkForTypeName("MongoDB CDC")).toBe(brandMarkForTypeName("MongoDB"))
    expect(brandMarkForTypeName("MongoDB CDC")?.title).toBe("MongoDB")
  })

  it("gives no mark to Oracle, SQL Server, SQL Server CDC, Object storage, SFTP and REST API", () => {
    for (const name of ["Oracle", "SQL Server", "SQL Server CDC", "Object storage", "SFTP", "REST API"]) {
      expect(brandMarkForTypeName(name)).toBeNull()
    }
  })

  it("gives no mark to an unknown name or a near miss", () => {
    for (const name of ["", "CDC", " CDC", "Snowflake", "PostgreSQL-compatible", "PostgreSQL CDC CDC", "PostgreSQL ", "MySQL Cluster", "constructor", "toString"]) {
      expect(brandMarkForTypeName(name)).toBeNull()
    }
  })

  it("compares names as served, with no case folding", () => {
    for (const name of ["postgresql", "POSTGRESQL", "Postgresql", "mysql", "mongodb", "kafka", "google sheets", "sap / erp", "mqtt", "PostgreSQL cdc"]) {
      expect(brandMarkForTypeName(name)).toBeNull()
    }
  })
})

describe("BRAND_MARKS", () => {
  const dir = process.env.BRAND_ICON_DIR

  it("flags exactly the marks that need the foreground in the dark theme: Kafka, MariaDB, MQTT, Confluence (1.05) and Zendesk (1.13), brand colour below 3:1 on the chip, and MySQL (a thin line drawing, 3.16:1 is too faint)", () => {
    const flagged = Object.values(BRAND_MARKS)
      .filter((m) => m.needsForegroundInDark)
      .map((m) => m.title)
      .sort()
    expect(flagged).toEqual(["Apache Kafka", "Confluence", "MQTT", "MariaDB", "MySQL", "Zendesk"])
  })

  it("holds a single well-formed brand colour and a path for every mark", () => {
    for (const m of Object.values(BRAND_MARKS)) {
      expect(m.hex).toMatch(/^#[0-9A-F]{6}$/)
      expect(m.paths.length).toBeGreaterThan(0)
      expect(m.viewBox).toMatch(/^0 0 \d+ \d+$/)
      for (const d of m.paths) expect(d.length).toBeGreaterThan(100)
    }
  })

  it.skipIf(!dir)("matches the path data of the downloaded SVG files byte for byte", () => {
    // MySQL comes from SVG Logos' `mysql-icon.svg` (two paths, 256 x 252);
    // the others from Simple Icons' `<slug>.svg` (one path, 24 x 24; the first seven
    // from release 16.34.0, Confluence, Google Analytics, HubSpot and Zendesk from 16.35.0).
    for (const [slug, m] of Object.entries(BRAND_MARKS)) {
      const file = slug === "mysql" ? "mysql-icon.svg" : `${slug}.svg`
      const svg = readFileSync(`${dir}/${file}`, "utf8")
      const ds = [...svg.matchAll(/ d="([^"]*)"/g)].map((x) => x[1])
      expect(ds).toEqual([...m.paths])
      expect(svg).toContain(`viewBox="${m.viewBox}"`)
    }
  })
})
