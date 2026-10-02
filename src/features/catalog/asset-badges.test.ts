// The header's three badges used to be constants. Each now says, on hover,
// what it rests on — including when that is nothing at all.
import { describe, expect, it } from "bun:test"
import { DATA_LAYER_LABEL } from "@/lib/status"
import { classificationTitle, healthTitle, layerTitle, NO_HEALTH_SIGNAL, tierTitle } from "./asset-badges"

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

  // A connector's table was labelled "Raw" beside a description, a storage
  // card and a lineage node that all called it Bronze.
  it("calls raw and curated Bronze by one layer name, and says how they differ", () => {
    expect(DATA_LAYER_LABEL.raw).toBe("Bronze (raw)")
    expect(DATA_LAYER_LABEL.bronze).toBe("Bronze (curated)")
    expect(layerTitle({ layer: "raw" })).toContain("as loaded")
    expect(layerTitle({ layer: "bronze" })).toContain("curated")
    expect(layerTitle({ layer: "gold" })).toContain("mart")
  })

  it("explains the tier by where the data sits", () => {
    expect(tierTitle({ tier: "warm", type: "iceberg-table" })).toContain("Iceberg")
    expect(tierTitle({ tier: "hot", type: "table" })).toContain("ClickHouse")
    expect(tierTitle({ tier: "warm", type: "view" })).toContain("view")
  })
})
