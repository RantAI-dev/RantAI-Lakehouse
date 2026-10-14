import { describe, expect, it } from "bun:test"
import { freshnessLine, publicationLabel } from "./gold-freshness"
import type { GoldPublication } from "@/services/contracts/gold"

const basePublication: GoldPublication = {
  mart: "sales_rollup",
  enabled: true,
  updatedAt: "2026-10-14T10:00:00Z",
  lastChangedAt: "2026-10-14T09:30:00Z",
  lastExportedAt: "2026-10-14T10:00:00Z",
  changedSinceExport: false,
  canEdit: true,
}

describe("gold-freshness", () => {
  it("reports Never published when lastExportedAt is null", () => {
    const pub: GoldPublication = {
      ...basePublication,
      lastExportedAt: null,
      changedSinceExport: null,
    }
    expect(freshnessLine(pub)).toBe("Never published")
    expect(publicationLabel(pub)).toBe("On · Never published")
  })

  it("reports Not measured when changedSinceExport is null", () => {
    const pub: GoldPublication = {
      ...basePublication,
      changedSinceExport: null,
    }
    expect(freshnessLine(pub)).toBe("Not measured")
    expect(publicationLabel(pub)).toBe("On · Not measured")
  })

  it("reports Up to date when changedSinceExport is false", () => {
    const pub: GoldPublication = {
      ...basePublication,
      changedSinceExport: false,
    }
    expect(freshnessLine(pub)).toBe("Up to date")
    expect(publicationLabel(pub)).toBe("On · Up to date")
  })

  it("reports Out of date when changedSinceExport is true", () => {
    const pub: GoldPublication = {
      ...basePublication,
      changedSinceExport: true,
    }
    expect(freshnessLine(pub)).toBe("Out of date")
    expect(publicationLabel(pub)).toBe("On · Out of date")
  })

  it("reports Off when enabled is false regardless of freshness", () => {
    const pub: GoldPublication = {
      ...basePublication,
      enabled: false,
      changedSinceExport: false,
    }
    expect(publicationLabel(pub)).toBe("Off")
  })
})
