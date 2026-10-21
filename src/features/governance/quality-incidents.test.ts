// DATA-14 (F7): the incident overview's grouping and the schedule sentences
// are plain functions, so the order a person reads them in is pinned here.
import { describe, expect, it } from "bun:test"
import type { QualityIncident } from "@/services/contracts/governance"
import { groupByTable, resolutionReason } from "./quality-incidents"
import { INTERVAL_CHOICES, intervalLabel, scheduleSentence } from "./quality-schedule"

const incident = (id: string, asset: string, openedAt: string): QualityIncident => ({
  id,
  asset,
  ruleId: null,
  ruleName: null,
  kind: "volume_anomaly",
  openedAt,
  lastSeenAt: openedAt,
  detail: "",
  status: "open",
  resolvedAt: null,
  resolvedBy: null,
  resolution: null,
})

describe("groupByTable", () => {
  it("puts the table with the oldest incident first, and each table's incidents oldest first", () => {
    const groups = groupByTable([
      incident("a", "silver.orders", "2026-10-15T09:00:00.000Z"),
      incident("b", "serving.mart", "2026-10-12T09:00:00.000Z"),
      incident("c", "silver.orders", "2026-10-11T09:00:00.000Z"),
    ])
    expect(groups.map((g) => g.asset)).toEqual(["silver.orders", "serving.mart"])
    expect(groups[0].oldest).toBe("2026-10-11T09:00:00.000Z")
    expect(groups[0].incidents.map((i) => i.id)).toEqual(["c", "a"])
    expect(groups[1].incidents.map((i) => i.id)).toEqual(["b"])
  })

  it("breaks a tie by table name and groups nothing into nothing", () => {
    const same = "2026-10-11T09:00:00.000Z"
    expect(groupByTable([incident("a", "silver.b", same), incident("b", "silver.a", same)]).map((g) => g.asset)).toEqual([
      "silver.a",
      "silver.b",
    ])
    expect(groupByTable([])).toEqual([])
  })
})

describe("resolutionReason", () => {
  it("is the trimmed text, or nothing when it is blank", () => {
    expect(resolutionReason("  fixed  ")).toBe("fixed")
    expect(resolutionReason("   ")).toBeNull()
    expect(resolutionReason("")).toBeNull()
  })
})

describe("the schedule sentences", () => {
  it("names intervals from an hour to a day", () => {
    expect(INTERVAL_CHOICES.map(intervalLabel)).toEqual([
      "1 hour",
      "2 hours",
      "3 hours",
      "6 hours",
      "12 hours",
      "1 day",
    ])
    expect(intervalLabel(90)).toBe("90 minutes")
  })

  it("says the default, a timer, and on every load with its backstop", () => {
    expect(scheduleSentence(undefined, 60)).toBe("Every 1 hour (the default).")
    const base = { asset: "silver.orders", updatedBy: null, updatedAt: "" }
    expect(scheduleSentence({ ...base, mode: "interval", intervalMinutes: 180 }, 60)).toBe("Every 3 hours.")
    expect(scheduleSentence({ ...base, mode: "on_load", intervalMinutes: 1440 }, 60)).toBe(
      "After every load of the table, and every 1 day as a backstop."
    )
  })
})
