import { describe, expect, it } from "bun:test"
import { parseReportName, reportName, reportProblem } from "./workday-report"

describe("a Workday report name", () => {
  it("is owner/report, trimmed", () => {
    expect(reportName(" isu_owner ", " Headcount_Report ")).toBe("isu_owner/Headcount_Report")
    expect(parseReportName("isu_owner/Headcount_Report")).toEqual({ owner: "isu_owner", report: "Headcount_Report" })
  })

  it("accepts what the server accepts and refuses what could leave the request path", () => {
    expect(reportProblem("a.b@x", "My_Report-1", [])).toBeNull()
    for (const [owner, report] of [
      ["", "Report"],
      ["owner", ""],
      ["o wner", "Report"],
      ["owner", "Report name"],
      ["owner", "a/b"],
      ["owner", "Report?x=1"],
      ["owner", "%2e%2e"],
      ["owner", "a..b"],
      ["owner", "a".repeat(151)],
      ["@owner", "Report"],
    ]) {
      expect(reportProblem(owner, report, [])).not.toBeNull()
    }
  })

  it("refuses a report that is already listed, and text that is not owner/report is not a report", () => {
    expect(reportProblem("o", "r", ["o/r"])).toContain("already")
    expect(parseReportName("Report")).toBeNull()
    expect(parseReportName("o/a/b")).toBeNull()
  })
})
