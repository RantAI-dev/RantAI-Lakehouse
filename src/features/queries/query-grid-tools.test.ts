// The grid's own arithmetic: what a cell reads as, which rows a filter
// keeps, what a column's statistics are and how a column is broken into
// bins. The rules are easy to get wrong and cheap to break, so they are
// tested here rather than through a rendered table.
import { describe, expect, it } from "bun:test"
import {
  cellText,
  columnStatistics,
  filterRows,
  histogram,
} from "./query-grid-tools"

describe("cellText", () => {
  it("reads a null cell as no text at all", () => {
    expect(cellText(null)).toBe("")
  })

  it("reads a number and a boolean as their own text", () => {
    expect(cellText(10)).toBe("10")
    expect(cellText(10.5)).toBe("10.5")
    expect(cellText(true)).toBe("true")
    expect(cellText(false)).toBe("false")
  })

  it("leaves a string as it is", () => {
    expect(cellText("Jakarta")).toBe("Jakarta")
  })
})

describe("filterRows", () => {
  const rows = [
    { city: "Jakarta", owner: "Ada" },
    { city: "Bandung", owner: "Grace" },
    { city: "Jakarta Selatan", owner: "Lin" },
  ]

  it("keeps the rows whose cell holds the text, whatever the case", () => {
    expect(filterRows(rows, { city: "jakarta" })).toEqual([rows[0], rows[2]])
  })

  it("combines two filters with AND, not with OR", () => {
    expect(filterRows(rows, { city: "jakarta", owner: "lin" })).toEqual([rows[2]])
    expect(filterRows(rows, { city: "bandung", owner: "lin" })).toEqual([])
  })

  it("keeps every row when no filter holds text", () => {
    expect(filterRows(rows, { city: "", owner: "" })).toEqual(rows)
  })

  it("matches a null cell only when the filter is empty", () => {
    const withNull = [{ city: null, owner: "Ada" }]
    expect(filterRows(withNull, { city: "a" })).toEqual([])
    expect(filterRows(withNull, { city: "" })).toEqual(withNull)
  })

  it("keeps the order the rows came in", () => {
    const many = [
      { city: "b" },
      { city: "a" },
      { city: "c" },
    ]
    expect(filterRows(many, { city: "" }).map((row) => row.city)).toEqual([
      "b",
      "a",
      "c",
    ])
  })
})

describe("columnStatistics", () => {
  it("counts the empty cells and the distinct values", () => {
    const stats = columnStatistics(
      [{ v: "a" }, { v: "a" }, { v: null }, { v: "" }, { v: "b" }],
      "v"
    )
    expect(stats.empty).toBe(2)
    expect(stats.distinct).toBe(2)
  })

  it("reads the smallest and the largest of a numeric column as numbers", () => {
    const stats = columnStatistics([{ v: 10 }, { v: 9 }, { v: 100 }], "v")
    expect(stats.numeric).toBe(true)
    expect(stats.min).toBe("9")
    expect(stats.max).toBe("100")
  })

  it("reads the smallest and the largest of a text column in text order", () => {
    const stats = columnStatistics([{ v: "b" }, { v: "a" }, { v: "c" }], "v")
    expect(stats.numeric).toBe(false)
    expect(stats.min).toBe("a")
    expect(stats.max).toBe("c")
  })

  it("has no smallest and no largest when the column holds no value", () => {
    const stats = columnStatistics([{ v: null }, { v: "" }], "v")
    expect(stats.min).toBeNull()
    expect(stats.max).toBeNull()
  })

  it("counts a column of numbers that arrived as text as numeric", () => {
    const stats = columnStatistics([{ v: "10" }, { v: "9" }], "v")
    expect(stats.numeric).toBe(true)
    expect(stats.min).toBe("9")
    expect(stats.max).toBe("10")
  })

  it("does not count a column of text as numeric", () => {
    const stats = columnStatistics([{ v: "10" }, { v: "nine" }], "v")
    expect(stats.numeric).toBe(false)
    expect(stats.min).toBe("10")
    expect(stats.max).toBe("nine")
  })

  it("counts a column that holds a number as numeric, the way the grid does", () => {
    // The grid right-aligns a column whose values arrive as numbers, so the
    // statistics read such a column as a numeric one too. The smallest and
    // the largest are the numbers in it.
    const stats = columnStatistics([{ v: 10 }, { v: "nine" }], "v")
    expect(stats.numeric).toBe(true)
    expect(stats.min).toBe("10")
    expect(stats.max).toBe("10")
  })

  it("reads the smallest and the largest of a column of more than 65,535 rows", () => {
    // The result grid holds up to 100,000 rows, so a column can be larger
    // than a browser accepts for a call that takes one argument per row.
    const rows = Array.from({ length: 70_000 }, (_, index) => ({ v: index }))
    const stats = columnStatistics(rows, "v")
    expect(stats.min).toBe("0")
    expect(stats.max).toBe("69999")
  })

  it("reads the smallest and the largest of a column of negative and fractional values", () => {
    const stats = columnStatistics(
      [{ v: 3.75 }, { v: -10 }, { v: 0 }, { v: -2.5 }],
      "v"
    )
    expect(stats.min).toBe("-10")
    expect(stats.max).toBe("3.75")
  })
})

describe("histogram", () => {
  it("counts the values of a text column, the most frequent first", () => {
    const shape = histogram([{ v: "a" }, { v: "b" }, { v: "a" }, { v: "c" }], "v")
    expect(shape.kind).toBe("values")
    if (shape.kind !== "values") throw new Error("expected the values shape")
    expect(shape.values).toEqual([
      { value: "a", count: 2 },
      { value: "b", count: 1 },
      { value: "c", count: 1 },
    ])
  })

  it("breaks a tie by the value's own text, so the order never moves", () => {
    const shape = histogram([{ v: "b" }, { v: "a" }], "v")
    if (shape.kind !== "values") throw new Error("expected the values shape")
    expect(shape.values.map((entry) => entry.value)).toEqual(["a", "b"])
  })

  it("leaves the empty cell out of a text column's values", () => {
    const shape = histogram([{ v: "a" }, { v: null }, { v: "" }], "v")
    if (shape.kind !== "values") throw new Error("expected the values shape")
    expect(shape.values).toEqual([{ value: "a", count: 1 }])
  })

  it("breaks a numeric column into ten equal-width bins", () => {
    const rows = Array.from({ length: 10 }, (_, index) => ({ v: index }))
    const shape = histogram(rows, "v")
    expect(shape.kind).toBe("bins")
    if (shape.kind !== "bins") throw new Error("expected the bins shape")
    expect(shape.bins).toHaveLength(10)
    expect(shape.bins[0]?.lower).toBe(0)
    expect(shape.bins[0]?.upper).toBeCloseTo(0.9, 10)
    expect(shape.bins[9]?.upper).toBe(9)
    expect(shape.bins.every((bin) => bin.count === 1)).toBe(true)
  })

  it("never makes more bins than there are distinct values", () => {
    const shape = histogram([{ v: 1 }, { v: 2 }], "v")
    if (shape.kind !== "bins") throw new Error("expected the bins shape")
    expect(shape.bins).toHaveLength(2)
  })

  it("puts a column that holds one value in one bin", () => {
    const shape = histogram([{ v: 5 }, { v: 5 }], "v")
    if (shape.kind !== "bins") throw new Error("expected the bins shape")
    expect(shape.bins).toEqual([{ lower: 5, upper: 5, count: 2 }])
  })

  it("counts every value of the column in one bin", () => {
    const shape = histogram([{ v: 1 }, { v: 2 }, { v: 3 }], "v")
    if (shape.kind !== "bins") throw new Error("expected the bins shape")
    const counted = shape.bins.reduce((sum, bin) => sum + bin.count, 0)
    expect(counted).toBe(3)
  })

  it("shows the ten most frequent values of a text column, not more", () => {
    const rows = Array.from({ length: 12 }, (_, index) => ({
      v: `value ${String(index).padStart(2, "0")}`,
    }))
    const shape = histogram(rows, "v")
    if (shape.kind !== "values") throw new Error("expected the values shape")
    expect(shape.values).toHaveLength(10)
  })

  it("bins a column of more than 65,535 rows between its smallest and its largest value", () => {
    // The result grid holds up to 100,000 rows, so a column can be larger
    // than a browser accepts for a call that takes one argument per row.
    const rows = Array.from({ length: 70_000 }, (_, index) => ({ v: index }))
    const shape = histogram(rows, "v")
    if (shape.kind !== "bins") throw new Error("expected the bins shape")
    expect(shape.bins[0]?.lower).toBe(0)
    expect(shape.bins[shape.bins.length - 1]?.upper).toBe(69999)
    const counted = shape.bins.reduce((sum, bin) => sum + bin.count, 0)
    expect(counted).toBe(70_000)
  })

  it("bins a column of negative and fractional values between its smallest and its largest value", () => {
    const shape = histogram(
      [{ v: 3.75 }, { v: -10 }, { v: 0 }, { v: -2.5 }],
      "v"
    )
    if (shape.kind !== "bins") throw new Error("expected the bins shape")
    expect(shape.bins[0]?.lower).toBe(-10)
    expect(shape.bins[shape.bins.length - 1]?.upper).toBe(3.75)
    const counted = shape.bins.reduce((sum, bin) => sum + bin.count, 0)
    expect(counted).toBe(4)
  })
})
