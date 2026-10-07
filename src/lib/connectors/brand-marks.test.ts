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

  it("gives a CDC type its database's mark", () => {
    expect(brandMarkForTypeName("PostgreSQL CDC")).toBe(brandMarkForTypeName("PostgreSQL"))
    expect(brandMarkForTypeName("MySQL CDC")).toBe(brandMarkForTypeName("MySQL"))
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

  it("flags exactly the marks that need the foreground in the dark theme: Kafka, MariaDB and MQTT (brand colour below 3:1 on the chip) and MySQL (a thin line drawing, 3.16:1 is too faint)", () => {
    const flagged = Object.values(BRAND_MARKS)
      .filter((m) => m.needsForegroundInDark)
      .map((m) => m.title)
      .sort()
    expect(flagged).toEqual(["Apache Kafka", "MQTT", "MariaDB", "MySQL"])
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
    // the other seven from Simple Icons' `<slug>.svg` (one path, 24 x 24).
    for (const [slug, m] of Object.entries(BRAND_MARKS)) {
      const file = slug === "mysql" ? "mysql-icon.svg" : `${slug}.svg`
      const svg = readFileSync(`${dir}/${file}`, "utf8")
      const ds = [...svg.matchAll(/ d="([^"]*)"/g)].map((x) => x[1])
      expect(ds).toEqual([...m.paths])
      expect(svg).toContain(`viewBox="${m.viewBox}"`)
    }
  })
})
