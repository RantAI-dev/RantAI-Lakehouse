// The header's three badges used to be constants. Each now says, on hover,
// what it rests on — including when that is nothing at all.
import { describe, expect, it } from "bun:test"
import { classificationTitle, healthTitle, NO_HEALTH_SIGNAL, tierTitle } from "./asset-badges"

describe("badge explanations", () => {
  it("lists the health signals, or says there are none", () => {
    expect(healthTitle({ healthReasons: ["Fresh: written 5h 30m ago, within its 36h 00m target", "2 of 2 quality checks passed"] })).toBe(
      "Fresh: written 5h 30m ago, within its 36h 00m target\n2 of 2 quality checks passed"
    )
    expect(healthTitle({ healthReasons: [] })).toBe(NO_HEALTH_SIGNAL)
    // An API build that sends no reasons at all.
    expect(healthTitle({})).toBe(NO_HEALTH_SIGNAL)
  })

  it("tells a rule's classification from the default", () => {
    expect(classificationTitle({ classificationSource: "rule" })).toContain("classification rule")
    expect(classificationTitle({ classificationSource: "default" })).toContain("default level")
    expect(classificationTitle({})).toContain("default level")
  })

  it("explains the tier by where the data sits", () => {
    expect(tierTitle({ tier: "warm", type: "iceberg-table" })).toContain("Iceberg")
    expect(tierTitle({ tier: "hot", type: "table" })).toContain("ClickHouse")
    expect(tierTitle({ tier: "warm", type: "view" })).toContain("view")
  })
})
