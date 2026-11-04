import { describe, expect, it } from "bun:test"
import { LIMIT_DEFAULTS, LIMIT_RANGES, limitProblem, limitSentence } from "./query-limits"
import { newRunId } from "./run-id"

// BI-5 (feature page: decisions 1, 2 and 5; plan D5).
describe("limitProblem", () => {
  it("accepts the ends of each range and the defaults", () => {
    for (const [field, r] of Object.entries(LIMIT_RANGES)) {
      expect(limitProblem(field as keyof typeof LIMIT_RANGES, String(r.min))).toBeNull()
      expect(limitProblem(field as keyof typeof LIMIT_RANGES, String(r.max))).toBeNull()
    }
    expect(limitProblem("timeoutSeconds", String(LIMIT_DEFAULTS.timeoutSeconds))).toBeNull()
  })

  it("says the range when a value is outside it, fractional or empty", () => {
    expect(limitProblem("timeoutSeconds", "9")).toBe("Time limit: a whole number from 10 to 1,800 seconds.")
    expect(limitProblem("timeoutSeconds", "1801")).toBe("Time limit: a whole number from 10 to 1,800 seconds.")
    expect(limitProblem("chartRows", "99")).toBe("Rows a chart reads: a whole number from 100 to 100,000 rows.")
    expect(limitProblem("mapPoints", "50001")).toBe("Points a map shows: a whole number from 100 to 50,000 points.")
    expect(limitProblem("studioRows", "1.5")).not.toBeNull()
    expect(limitProblem("studioRows", "")).not.toBeNull()
  })

  it("keeps the feature page's defaults", () => {
    expect(LIMIT_DEFAULTS).toEqual({ timeoutSeconds: 60, chartRows: 2000, mapPoints: 5000, studioRows: 64000 })
  })
})

describe("limitSentence", () => {
  it("names the time and whose limit it was", () => {
    expect(limitSentence({ kind: "time", value: 30, source: { kind: "role", name: "Analyst" } })).toBe("This chart stopped after 30 seconds, the limit for your role.")
    expect(limitSentence({ kind: "time", value: 60, source: { kind: "install" } })).toBe("This chart stopped after 60 seconds, the limit for this install.")
    expect(limitSentence({ kind: "time", value: 1, source: { kind: "install" } })).toContain("after 1 second,")
  })

  it("names the rows and the administrator", () => {
    expect(limitSentence({ kind: "rows", value: 2000, source: { kind: "install" } })).toBe("Showing the first 2,000 rows, the limit set by your administrator.")
  })
})

describe("newRunId", () => {
  it("is a UUID v4, different each time", () => {
    const a = newRunId()
    expect(a).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/)
    expect(newRunId()).not.toBe(a)
  })
})
