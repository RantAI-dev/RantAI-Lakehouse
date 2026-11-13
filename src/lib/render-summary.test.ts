import { describe, expect, it } from "bun:test"
import { filterLine, issuedLine, paperContentWidth, paramLines } from "./render-summary"

describe("the PDF's first page lines", () => {
  it("writes each kind of filter as one line", () => {
    expect(filterLine({ column: "region", values: ["north", "south"] })).toBe("region: north, south")
    expect(filterLine({ column: "region", values: ["x"], op: "not_in" })).toBe("region not in: x")
    expect(filterLine({ column: "visits", values: [], op: "between", min: "10", max: "20" })).toBe("visits: 10 to 20")
    expect(filterLine({ column: "day", values: [], op: "between", min: "2026-01-01" })).toBe("day: 2026-01-01 to …")
    expect(filterLine({ column: "day", values: [], op: "relative", anchor: "last", unit: "day", n: 7 })).toBe("day: last 7 days")
    expect(filterLine({ column: "day", values: [], op: "relative", anchor: "this", unit: "month" })).toBe("day: this month")
    expect(filterLine({ column: "name", values: [], op: "starts_with", text: "Bal" })).toBe("name starts with “Bal”")
    expect(filterLine({ column: "", values: [], segment: "vip" })).toBe("Segment vip")
  })

  it("lists the parameters by label with the value used", () => {
    expect(paramLines(undefined)).toEqual([])
    expect(
      paramLines([
        { param: { id: "1", name: "region", label: "Region", type: "text", default: "all" }, value: "north" },
        { param: { id: "2", name: "top", label: "", type: "number", default: 5 }, value: 10 },
      ]),
    ).toEqual(["Region: north", "top: 10"])
  })

  // BI-53 (D17): the print view is laid out at the paper's printable width.
  it("the printable width is the paper's width less the two side margins, in pixels", () => {
    expect(paperContentWidth("a4", "portrait")).toBe(697)
    expect(paperContentWidth("a4", "landscape")).toBe(1027)
    expect(paperContentWidth("letter", "portrait")).toBe(720)
    expect(paperContentWidth("letter", "landscape")).toBe(960)
    expect(paperContentWidth(undefined, undefined)).toBe(697)
  })

  it("writes the time in the report zone, and falls back to UTC for a zone it does not know", () => {
    // 2026-10-17 07:05 UTC is 14:05 in Jakarta (UTC+7).
    const at = Date.UTC(2026, 9, 17, 7, 5) / 1000
    expect(issuedLine(at, "Asia/Jakarta", "en-US")).toContain("14:05")
    expect(issuedLine(at, "Nowhere/Land", "en-US")).toBe("2026-10-17 07:05 UTC")
    expect(issuedLine(undefined, "UTC")).toBe("")
  })
})
