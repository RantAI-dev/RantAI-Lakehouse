// UX-8: the trust strip's four facts, from the asset alone. Each says what
// it can measure, a dash for what nothing measures, and the tab that holds
// the rest.
import { describe, expect, it } from "bun:test"
import type { AssetDetail } from "@/services/contracts/assets"
import { trustFacts } from "./asset-header"

const BASE = {
  lastUpdated: null,
  freshnessLagSeconds: null,
  freshnessTargetSeconds: null,
  qualityChecks: [],
  usage: null,
  schema: [{ name: "id", dataType: "Int64" }],
} as unknown as AssetDetail

const value = (a: AssetDetail, key: string, canRead = true) =>
  trustFacts(a, canRead).find((f) => f.key === key)?.value

describe("trustFacts", () => {
  it("says a dash for what nothing measures, and no checks for an unchecked table", () => {
    expect(value(BASE, "updated")).toBe("—")
    expect(value(BASE, "used")).toBe("—")
    expect(value(BASE, "quality")).toBe("No checks")
  })

  it("counts the checks that ran, and names the failures before the warnings", () => {
    const a = {
      ...BASE,
      qualityChecks: [
        { status: "passed", lastRun: "2026-11-20T00:00:00Z" },
        { status: "warning", lastRun: "2026-11-20T00:00:00Z" },
        { status: "failed", lastRun: "2026-11-20T00:00:00Z" },
        { status: null, lastRun: null },
      ],
    } as unknown as AssetDetail
    expect(value(a, "quality")).toBe("1 of 3 passed · 1 failed")
    expect(trustFacts(a, true).find((f) => f.key === "quality")?.tone).toBe("bad")
  })

  it("says it is late when the age is past its target", () => {
    const a = { ...BASE, lastUpdated: new Date().toISOString(), freshnessLagSeconds: 7200, freshnessTargetSeconds: 3600 } as unknown as AssetDetail
    expect(value(a, "updated")).toMatch(/ · late$/)
  })

  it("counts the queries of the usage window with a thousands separator", () => {
    const a = { ...BASE, usage: { days: 30, queries: 1571 } } as unknown as AssetDetail
    expect(value(a, "used")).toBe("1,571 queries in 30 days")
  })

  it("says what the reader may do, and how many columns are masked for them", () => {
    const masked = { ...BASE, schema: [{ name: "email", dataType: "String", masked: true }] } as unknown as AssetDetail
    expect(value(masked, "access")).toBe("You can read · 1 masked")
    expect(value(BASE, "access")).toBe("You can read · nothing masked")
    expect(value(BASE, "access", false)).toBe("You can see it, not read its rows")
  })

  it("points each fact at its tab", () => {
    expect(trustFacts(BASE, true).map((f) => [f.key, f.tab])).toEqual([
      ["updated", "activity"],
      ["quality", "quality"],
      ["used", "activity"],
      ["access", "access"],
    ])
  })
})
