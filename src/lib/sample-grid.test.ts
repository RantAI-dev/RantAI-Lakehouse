import { describe, expect, it } from "bun:test"
import { cellKind, nextSort, sortedOrder, sortRows, type SortState } from "./sample-grid"

type Row = Record<string, string | null>

/** The values of `column`, in the order the rows came back. */
const column = (rows: Row[], name: string) => rows.map((r) => r[name])

describe("cellKind", () => {
  it("tells no value, an empty text and a value apart", () => {
    expect(cellKind(null)).toBe("null")
    expect(cellKind("")).toBe("empty")
    expect(cellKind("0")).toBe("value")
    expect(cellKind("NULL")).toBe("value")
    expect(cellKind(" ")).toBe("value")
  })

  it("reads a cell the row does not carry as no value", () => {
    expect(cellKind(undefined)).toBe("null")
  })
})

describe("nextSort", () => {
  it("goes none, ascending, descending, none on one column", () => {
    const first = nextSort(null, "amount")
    expect(first).toEqual({ column: "amount", direction: "asc" })
    const second = nextSort(first, "amount")
    expect(second).toEqual({ column: "amount", direction: "desc" })
    expect(nextSort(second, "amount")).toBeNull()
  })

  it("starts a different column at ascending, whatever the first was doing", () => {
    const desc: SortState = { column: "amount", direction: "desc" }
    expect(nextSort(desc, "city")).toEqual({ column: "city", direction: "asc" })
    const asc: SortState = { column: "amount", direction: "asc" }
    expect(nextSort(asc, "city")).toEqual({ column: "city", direction: "asc" })
  })
})

describe("sortRows", () => {
  it("compares the values of a number column as numbers, so 10 comes after 9", () => {
    const rows: Row[] = [{ n: "10" }, { n: "9" }, { n: "100" }, { n: "-5" }, { n: "2.5" }]
    expect(column(sortRows(rows, "n", "asc", "number"), "n")).toEqual(["-5", "2.5", "9", "10", "100"])
    expect(column(sortRows(rows, "n", "desc", "number"), "n")).toEqual(["100", "10", "9", "2.5", "-5"])
  })

  it("compares the values of any other column as text, even when they look like numbers", () => {
    const rows: Row[] = [{ n: "10" }, { n: "9" }, { n: "100" }]
    expect(column(sortRows(rows, "n", "asc", "text"), "n")).toEqual(["10", "100", "9"])
    // A column the schema does not list has no family: never guessed from the values.
    expect(column(sortRows(rows, "n", "asc", undefined), "n")).toEqual(["10", "100", "9"])
  })

  it("sorts text the way a reader expects, not by letter case", () => {
    const rows: Row[] = [{ s: "cherry" }, { s: "Banana" }, { s: "apple" }]
    expect(column(sortRows(rows, "s", "asc", "text"), "s")).toEqual(["apple", "Banana", "cherry"])
    expect(column(sortRows(rows, "s", "desc", "text"), "s")).toEqual(["cherry", "Banana", "apple"])
  })

  it("sorts dates and times stored as text by their text", () => {
    const rows: Row[] = [{ at: "2026-09-23 04:57:42" }, { at: "2026-09-02 10:00:00" }, { at: "2025-12-31 23:59:59" }]
    expect(column(sortRows(rows, "at", "asc", "time"), "at")).toEqual([
      "2025-12-31 23:59:59",
      "2026-09-02 10:00:00",
      "2026-09-23 04:57:42",
    ])
  })

  it("keeps the numbers of a number column ahead of a value that does not parse, and sorts that value as text", () => {
    const rows: Row[] = [{ n: "***" }, { n: "10" }, { n: "n/a" }, { n: "9" }]
    expect(column(sortRows(rows, "n", "asc", "number"), "n")).toEqual(["9", "10", "***", "n/a"])
    expect(column(sortRows(rows, "n", "desc", "number"), "n")).toEqual(["n/a", "***", "10", "9"])
  })

  it("gives a mixed number column one consistent order, where pairwise text and number rules would loop", () => {
    // 9 < 10 as numbers, 10 < 1e and 1e < 9 as text: no order satisfies all three.
    const rows: Row[] = [{ n: "1e" }, { n: "9" }, { n: "10" }]
    expect(column(sortRows(rows, "n", "asc", "number"), "n")).toEqual(["9", "10", "1e"])
  })

  it("reads exponent and signed forms as numbers", () => {
    const rows: Row[] = [{ n: "1.5e3" }, { n: "+7" }, { n: "-2e-1" }, { n: ".5" }]
    expect(column(sortRows(rows, "n", "asc", "number"), "n")).toEqual(["-2e-1", ".5", "+7", "1.5e3"])
  })

  it("keeps the last digits of a 64-bit id, which a double would tie with its neighbour", () => {
    const rows: Row[] = [{ id: "9223372036854775807" }, { id: "9223372036854775806" }, { id: "9223372036854775805" }]
    expect(column(sortRows(rows, "id", "asc", "number"), "id")).toEqual([
      "9223372036854775805",
      "9223372036854775806",
      "9223372036854775807",
    ])
  })

  it("puts NULL last in both directions, and sorts an empty text with the texts", () => {
    const rows: Row[] = [{ s: null }, { s: "b" }, { s: "" }, { s: "a" }, { s: null }]
    expect(column(sortRows(rows, "s", "asc", "text"), "s")).toEqual(["", "a", "b", null, null])
    expect(column(sortRows(rows, "s", "desc", "text"), "s")).toEqual(["b", "a", "", null, null])
  })

  it("puts NULL last in a number column too", () => {
    const rows: Row[] = [{ n: null }, { n: "3" }, { n: "20" }]
    expect(column(sortRows(rows, "n", "asc", "number"), "n")).toEqual(["3", "20", null])
    expect(column(sortRows(rows, "n", "desc", "number"), "n")).toEqual(["20", "3", null])
  })

  it("treats a row that lacks the column as having no value there", () => {
    const rows: Row[] = [{ other: "x" }, { s: "b" }, { s: "a" }]
    expect(sortRows(rows, "s", "asc", "text").map((r) => r.s ?? null)).toEqual(["a", "b", null])
  })

  it("lets the table's order break ties, in either direction", () => {
    const rows: Row[] = [
      { k: "b", at: "first" },
      { k: "a", at: "second" },
      { k: "b", at: "third" },
      { k: "a", at: "fourth" },
      { k: null, at: "fifth" },
      { k: null, at: "sixth" },
    ]
    expect(column(sortRows(rows, "k", "asc", "text"), "at")).toEqual([
      "second",
      "fourth",
      "first",
      "third",
      "fifth",
      "sixth",
    ])
    expect(column(sortRows(rows, "k", "desc", "text"), "at")).toEqual([
      "first",
      "third",
      "second",
      "fourth",
      "fifth",
      "sixth",
    ])
  })

  it("treats 1 and 1.0 as equal numbers, so the table's order decides", () => {
    const rows: Row[] = [{ n: "1.0", at: "first" }, { n: "1", at: "second" }, { n: "0", at: "third" }]
    expect(column(sortRows(rows, "n", "asc", "number"), "at")).toEqual(["third", "first", "second"])
  })

  it("returns a new array and leaves the input as it was", () => {
    const rows: Row[] = [{ n: "2" }, { n: "1" }]
    const before = JSON.stringify(rows)
    const sorted = sortRows(rows, "n", "asc", "number")
    expect(sorted).not.toBe(rows)
    expect(JSON.stringify(rows)).toBe(before)
    // The rows themselves are the same objects: only their order is new.
    expect(sorted[0]).toBe(rows[1])
  })

  it("sorts an empty list, and a list of one", () => {
    expect(sortRows([], "n", "asc", "number")).toEqual([])
    expect(sortRows([{ n: "1" }], "n", "desc", "number")).toEqual([{ n: "1" }])
  })
})

describe("sortedOrder", () => {
  it("gives each row's place in the table's own order, in the order the sort puts them", () => {
    const rows: Row[] = [{ n: "10" }, { n: "9" }, { n: null }, { n: "100" }]
    expect(sortedOrder(rows, "n", "asc", "number")).toEqual([1, 0, 3, 2])
    expect(sortedOrder(rows, "n", "desc", "number")).toEqual([3, 0, 1, 2])
  })
})
