import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import type { ConnectorType } from "@/services/contracts/connectors"
import { ConnectorTypePicker, SelectedTypeSummary } from "./connector-type-picker"
import { brandMarkForTypeName } from "@/lib/connectors/brand-marks"

afterEach(cleanup)

const TYPES: ConnectorType[] = [
  { name: "PostgreSQL", adapter: "sql", supported: true, docsUrl: null },
  { name: "Oracle", adapter: "sql", supported: true, docsUrl: null },
  { name: "PostgreSQL CDC", adapter: "cdc", supported: true, docsUrl: null },
  { name: "SQL Server CDC", adapter: "cdc", supported: true, docsUrl: null },
  { name: "SAP / ERP", adapter: null, supported: false, docsUrl: null },
]

function tile(name: string) {
  // Name is exact: "PostgreSQL" must not also match "PostgreSQL CDC".
  const found = screen.getAllByRole("radio").filter((el) => el.querySelector("span.truncate")?.textContent === name)
  expect(found).toHaveLength(1)
  return found[0]
}

const pgPath = brandMarkForTypeName("PostgreSQL")!.paths[0]

describe("ConnectorTypePicker marks", () => {
  it("shows the PostgreSQL mark on its tile and not the database cylinder", () => {
    render(<ConnectorTypePicker types={TYPES} value={null} onChange={() => {}} />)
    const svg = tile("PostgreSQL").querySelector("svg[data-brand-mark]")
    expect(svg?.querySelector("path")?.getAttribute("d")).toBe(pgPath)
    expect(tile("PostgreSQL").querySelector("svg.lucide-database")).toBeNull()
  })

  it("keeps the cylinder on an Oracle tile", () => {
    render(<ConnectorTypePicker types={TYPES} value={null} onChange={() => {}} />)
    expect(tile("Oracle").querySelector("svg.lucide-database")).not.toBeNull()
    expect(tile("Oracle").querySelector("svg[data-brand-mark]")).toBeNull()
  })

  it("shows a CDC tile its database's mark and SQL Server CDC the generic CDC icon", () => {
    render(<ConnectorTypePicker types={TYPES} value={null} onChange={() => {}} />)
    expect(tile("PostgreSQL CDC").querySelector("svg[data-brand-mark] path")?.getAttribute("d")).toBe(pgPath)
    expect(tile("PostgreSQL CDC").querySelector("svg.lucide-git-compare-arrows")).toBeNull()
    expect(tile("SQL Server CDC").querySelector("svg[data-brand-mark]")).toBeNull()
    expect(tile("SQL Server CDC").querySelector("svg.lucide-git-compare-arrows")).not.toBeNull()
  })

  it("shows an unsupported type with a mark (SAP) its mark, on a dimmed disabled tile", () => {
    render(<ConnectorTypePicker types={TYPES} value={null} onChange={() => {}} />)
    const sap = tile("SAP / ERP")
    expect(sap.hasAttribute("disabled")).toBe(true)
    expect(sap.querySelector("svg[data-brand-mark]")).not.toBeNull()
    expect(sap.querySelector("svg.lucide-plug")).toBeNull()
  })

  it("hides a mark from assistive technology and draws it in the brand colour on an unselected tile", () => {
    render(<ConnectorTypePicker types={TYPES} value={null} onChange={() => {}} />)
    const svg = tile("PostgreSQL").querySelector("svg[data-brand-mark]")!
    expect(svg.getAttribute("aria-hidden")).toBe("true")
    expect(svg.querySelector("title")).toBeNull()
    expect(svg.getAttribute("data-tone")).toBe("brand")
    expect(svg.getAttribute("style")).toContain("#4169E1")
    expect(svg.getAttribute("fill")).toBe("currentColor")
  })

  it("draws a mark whose brand colour is too dark for the dark theme in the foreground there", () => {
    render(<ConnectorTypePicker types={[{ name: "MariaDB", adapter: "sql", supported: true, docsUrl: null }]} value={null} onChange={() => {}} />)
    const svg = tile("MariaDB").querySelector("svg[data-brand-mark]")!
    expect(svg.getAttribute("class")).toContain("dark:text-foreground")
    cleanup()
    render(<ConnectorTypePicker types={TYPES} value={null} onChange={() => {}} />)
    expect(tile("PostgreSQL").querySelector("svg[data-brand-mark]")!.getAttribute("class")).not.toContain("dark:text-foreground")
  })

  it("takes the chip's foreground, not a brand colour, on the selected tile", () => {
    render(<ConnectorTypePicker types={TYPES} value="PostgreSQL" onChange={() => {}} />)
    const svg = tile("PostgreSQL").querySelector("svg[data-brand-mark]")!
    expect(svg.getAttribute("data-tone")).toBe("inherit")
    expect(svg.getAttribute("style")).toBeNull()
    expect(svg.getAttribute("class") ?? "").not.toContain("text-")
    expect(svg.parentElement?.className).toContain("text-primary-foreground")
  })
})

describe("MySQL mark", () => {
  it("draws both paths of the dolphin on its own non-square view box, in the brand colour and the foreground in dark", () => {
    render(<ConnectorTypePicker types={[{ name: "MySQL CDC", adapter: "cdc", supported: true, docsUrl: null }]} value={null} onChange={() => {}} />)
    const svg = tile("MySQL CDC").querySelector("svg[data-brand-mark]")!
    expect(svg.getAttribute("viewBox")).toBe("0 0 256 252")
    expect(svg.querySelectorAll("path")).toHaveLength(2)
    expect(svg.getAttribute("style")).toContain("#4479A1")
    expect(svg.getAttribute("class")).toContain("dark:text-foreground")
  })
})

describe("SelectedTypeSummary marks", () => {
  it("shows the same mark as the tile, in the chip's foreground", () => {
    render(<SelectedTypeSummary type={TYPES[0]} />)
    const svg = document.querySelector("svg[data-brand-mark]")!
    expect(svg.querySelector("path")?.getAttribute("d")).toBe(pgPath)
    expect(svg.getAttribute("data-tone")).toBe("inherit")
    expect(svg.getAttribute("aria-hidden")).toBe("true")
    expect(svg.parentElement?.className).toContain("text-primary-foreground")
  })

  it("shows the generic icon for a type without a mark", () => {
    render(<SelectedTypeSummary type={TYPES[1]} />)
    expect(document.querySelector("svg[data-brand-mark]")).toBeNull()
    expect(document.querySelector("svg.lucide-database")).not.toBeNull()
  })
})
