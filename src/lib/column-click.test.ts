import { describe, expect, it } from "bun:test"

import { autoUse, columnGroup, menuUses, type ColumnClick } from "./column-click"

const all = { x: true, value: true, split: true, filter: true }
const click = (patch: Partial<ColumnClick>): ColumnClick => ({ group: "text", offers: all, xEmpty: true, valueEmpty: true, inUse: false, ...patch })

describe("column-click (BI-58 T2)", () => {
  it("groups a column by its kind, a timestamp with the dates and an unknown one with the text", () => {
    expect(columnGroup("date")).toBe("date")
    expect(columnGroup("datetime")).toBe("date")
    expect(columnGroup("number")).toBe("number")
    expect(columnGroup("text")).toBe("text")
    expect(columnGroup(undefined)).toBe("text")
  })

  it("makes a date or text column the X axis while the chart has none", () => {
    expect(autoUse(click({ group: "date" }))).toBe("x")
    expect(autoUse(click({ group: "text" }))).toBe("x")
    expect(autoUse(click({ group: "date", xEmpty: false }))).toBeNull()
  })

  it("makes a number column the value while the chart has none", () => {
    expect(autoUse(click({ group: "number" }))).toBe("value")
    expect(autoUse(click({ group: "number", valueEmpty: false }))).toBeNull()
    // An empty X axis does not take a number on a click: numbers go to the value.
    expect(autoUse(click({ group: "number", valueEmpty: false, xEmpty: true }))).toBeNull()
  })

  it("opens the menu for a column in use, or one the empty slot does not offer", () => {
    expect(autoUse(click({ group: "date", inUse: true }))).toBeNull()
    expect(autoUse(click({ group: "text", offers: { ...all, x: false } }))).toBeNull()
    expect(autoUse(click({ group: "number", offers: { ...all, value: false } }))).toBeNull()
  })

  it("lists only the uses the chart offers, in a fixed order", () => {
    expect(menuUses(click({})).map((u) => u.label)).toEqual(["Use as X axis", "Use as value", "Use as split", "Use as filter"])
    expect(menuUses(click({ offers: { x: false, value: true, split: false, filter: true } })).map((u) => u.use)).toEqual(["value", "filter"])
  })
})
