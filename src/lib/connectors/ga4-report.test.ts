import { describe, expect, it } from "bun:test"
import { cursorOf, parseReportText, reportProblem, reportText, withTimeDimension } from "./ga4-report"

describe("a GA4 report text", () => {
  it("is written as the server reads it, with the date added when no time dimension is chosen", () => {
    expect(reportText({ label: " traffic ", dimensions: ["country"], metrics: ["sessions", "activeUsers"] })).toBe(
      "traffic = country, date | sessions, activeUsers"
    )
    expect(reportText({ label: "m", dimensions: ["yearMonth", "country"], metrics: ["sessions"] })).toBe("m = yearMonth, country | sessions")
  })

  it("has the first time dimension as its cursor", () => {
    expect(cursorOf(["country"])).toBe("date")
    expect(cursorOf(["country", "yearMonth"])).toBe("yearMonth")
    expect(withTimeDimension(["dateHour"])).toEqual(["dateHour"])
  })

  it("round-trips, and text that is not a report is null", () => {
    const report = { label: "t", dimensions: ["country", "date"], metrics: ["sessions", "customEvent:plan"] }
    expect(parseReportText(reportText(report))).toEqual(report)
    for (const text of ["", "country | sessions", "t = country sessions", "t = country |", "1t = a | b", "t = a b | c"]) {
      expect(parseReportText(text)).toBeNull()
    }
  })

  it("says why a report cannot be saved", () => {
    const ok = { label: "t", dimensions: ["country"], metrics: ["sessions"] }
    expect(reportProblem(ok, [])).toBeNull()
    expect(reportProblem({ ...ok, label: "1 bad" }, [])).toMatch(/Name the report/)
    expect(reportProblem(ok, ["t"])).toMatch(/exists/)
    expect(reportProblem({ ...ok, metrics: [] }, [])).toMatch(/at least one metric/)
    // PR #216 final review SHOULD-FIX 22: the Data API's two limits, separate, with the time
    // dimension counted (9 dimensions and 10 metrics, not 20 names).
    const tenMetrics = Array.from({ length: 10 }, (_, i) => `m${i}`)
    expect(reportProblem({ ...ok, metrics: tenMetrics }, [])).toBeNull()
    expect(reportProblem({ ...ok, metrics: [...tenMetrics, "m10"] }, [])).toMatch(/at most 10 metrics/)
    const nineDimensions = ["date", ...Array.from({ length: 8 }, (_, i) => `dim${i}`)]
    const eightMetrics = tenMetrics.slice(0, 8)
    expect(reportProblem({ label: "t", dimensions: nineDimensions, metrics: eightMetrics }, [])).toBeNull()
    expect(reportProblem({ label: "t", dimensions: [...nineDimensions, "extra"], metrics: eightMetrics }, [])).toMatch(
      /at most 9 dimensions/
    )
    expect(reportProblem({ ...ok, dimensions: ["a;b"] }, [])).toMatch(/does not use/)
  })
})
