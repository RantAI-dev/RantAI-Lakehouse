import { describe, expect, test } from "bun:test"
import type { CondRule } from "./chart-style"
import { conditionalStyle, domainsOf, readableOn } from "./conditional-format"

const below1000: CondRule = { column: "revenue", rule: { type: "compare", op: "lt", value: 1000 }, color: "#ff0000" }

describe("conditionalStyle", () => {
  test("below 1,000 paints the cell red with a readable text", () => {
    expect(conditionalStyle([below1000], "revenue", 500)).toEqual({ background: "#ff0000", color: "#000000" })
    expect(conditionalStyle([below1000], "revenue", 1000)).toBeUndefined()
    expect(conditionalStyle([below1000], "revenue", "250")).toEqual({ background: "#ff0000", color: "#000000" })
    expect(conditionalStyle([below1000], "revenue", "n/a")).toBeUndefined()
  })

  test("a rule reads only its own column", () => {
    expect(conditionalStyle([below1000], "cost", 5)).toBeUndefined()
  })

  test("every operator", () => {
    const ops: Record<string, readonly number[]> = { lt: [4, 5, 6], le: [4, 5, 6], gt: [4, 5, 6], ge: [4, 5, 6], eq: [4, 5, 6], ne: [4, 5, 6] }
    const expected: Record<string, number[]> = { lt: [1, 0, 0], le: [1, 1, 0], gt: [0, 0, 1], ge: [0, 1, 1], eq: [0, 1, 0], ne: [1, 0, 1] }
    for (const [op, values] of Object.entries(ops)) {
      const rule: CondRule = { column: "c", rule: { type: "compare", op: op as "lt", value: 5 }, color: "#00ff00" }
      const got: number[] = values.map((v) => (conditionalStyle([rule], "c", v) ? 1 : 0))
      expect(got, op).toEqual(expected[op])
    }
  })

  test("between is inclusive and contains ignores case", () => {
    const between: CondRule = { column: "c", rule: { type: "between", from: 1, to: 3 }, color: "#112233" }
    expect(conditionalStyle([between], "c", 3)).toBeDefined()
    expect(conditionalStyle([between], "c", 3.1)).toBeUndefined()
    const contains: CondRule = { column: "c", rule: { type: "contains", text: "ERR" }, color: "#112233", target: "text" }
    expect(conditionalStyle([contains], "c", "an error")).toEqual({ color: "#112233" })
    expect(conditionalStyle([contains], "c", null)).toBeUndefined()
  })

  test("the first rule that holds wins", () => {
    const green: CondRule = { column: "c", rule: { type: "compare", op: "lt", value: 10 }, color: "#00ff00" }
    const red: CondRule = { column: "c", rule: { type: "compare", op: "lt", value: 100 }, color: "#ff0000" }
    expect(conditionalStyle([green, red], "c", 5)?.background).toBe("#00ff00")
    expect(conditionalStyle([green, red], "c", 50)?.background).toBe("#ff0000")
  })

  test("a range runs from the first to the last colour over the shown minimum and maximum", () => {
    const rows = [{ c: 10 }, { c: 20 }, { c: 30 }]
    const rule: CondRule = { column: "c", rule: { type: "range", colors: ["#000000", "#ffffff"] } }
    const domains = domainsOf([rule], rows)
    expect(domains).toEqual({ c: { min: 10, max: 30 } })
    expect(conditionalStyle([rule], "c", 10, domains)?.background).toBe("#000000")
    expect(conditionalStyle([rule], "c", 20, domains)?.background).toBe("#808080")
    expect(conditionalStyle([rule], "c", 30, domains)?.background).toBe("#ffffff")
    expect(conditionalStyle([rule], "c", 5, {})).toBeUndefined()
  })

  test("a KPI passes no column: its one value is read by any rule", () => {
    const goal: CondRule = { column: "visitors", rule: { type: "compare", op: "gt", value: 1000 }, color: "#16a34a", target: "text" }
    expect(conditionalStyle([goal], null, 1500)).toEqual({ color: "#16a34a" })
    expect(conditionalStyle([goal], null, 900)).toBeUndefined()
  })

  test("text on a light background is black and on a dark one white", () => {
    expect(readableOn("#ffff00")).toBe("#000000")
    expect(readableOn("#000080")).toBe("#ffffff")
  })
})
