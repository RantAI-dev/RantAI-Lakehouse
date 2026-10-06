import { describe, expect, it } from "bun:test"
import type { ColumnProfile } from "@/services/contracts/assets"
import { formatShare, noValuesLabel, sortKeyColumns, valueShares } from "./column-profile"

const column = (over: Partial<ColumnProfile> = {}): ColumnProfile => ({
  name: "city",
  dataType: "String",
  profiled: true,
  nullCount: 100,
  nullFraction: 0.1,
  distinctCount: 4,
  topValues: [
    { value: "Jakarta", count: 500 },
    { value: "Bandung", count: 300 },
  ],
  ...over,
})

describe("valueShares", () => {
  it("gives each listed value its share of the rows profiled, in the profile's order", () => {
    const shares = valueShares(column(), 1000)
    expect(shares?.values).toEqual([
      { value: "Jakarta", count: 500, share: 0.5 },
      { value: "Bandung", count: 300, share: 0.3 },
    ])
  })

  it("adds up to the whole: the listed values, the rest and the nulls", () => {
    const shares = valueShares(column(), 1000)
    expect(shares?.nullShare).toBeCloseTo(0.1, 12)
    expect(shares?.otherShare).toBeCloseTo(0.1, 12)
    expect(shares?.nullCount).toBe(100)
    expect(shares?.otherCount).toBe(100)
    const total = (shares?.values.reduce((s, v) => s + v.share, 0) ?? 0) + (shares?.otherShare ?? 0) + (shares?.nullShare ?? 0)
    expect(total).toBeCloseTo(1, 12)
  })

  it("leaves nothing for the rest when the listed values and the nulls take every row", () => {
    const shares = valueShares(
      column({ nullCount: 400, nullFraction: 0.4, topValues: [{ value: "a", count: 600 }] }),
      1000
    )
    expect(shares?.otherShare).toBe(0)
    expect(shares?.otherCount).toBe(0)
  })

  it("does not turn the float noise of adding fractions into a sliver of 'other'", () => {
    // 0.1 + 0.2 + 0.7 is 0.9999999999999999 in floating point.
    const shares = valueShares(
      column({
        nullCount: 0,
        nullFraction: 0,
        topValues: [
          { value: "a", count: 100 },
          { value: "b", count: 200 },
          { value: "c", count: 700 },
        ],
      }),
      1000
    )
    expect(shares?.otherShare).toBe(0)
  })

  it("cuts a count above the rows profiled to the track, and never passes the whole", () => {
    const shares = valueShares(
      column({
        nullFraction: 0.5,
        topValues: [
          { value: "a", count: 5000 },
          { value: "b", count: 10 },
        ],
      }),
      1000
    )
    expect(shares?.values.map((v) => v.share)).toEqual([1, 0])
    // The count shown stays what the profile said.
    expect(shares?.values[0].count).toBe(5000)
    expect(shares?.nullShare).toBe(0)
    expect(shares?.otherShare).toBe(0)
  })

  it("never lets listed values and nulls together pass 1", () => {
    const shares = valueShares(
      column({ nullFraction: 0.6, topValues: [{ value: "a", count: 700 }] }),
      1000
    )
    expect(shares?.values[0].share).toBe(0.7)
    expect(shares?.nullShare).toBeCloseTo(0.3, 12)
    expect(shares?.otherShare).toBe(0)
  })

  it("is null when no rows were profiled", () => {
    expect(valueShares(column(), 0)).toBeNull()
    expect(valueShares(column(), Number.NaN)).toBeNull()
  })

  it("is null for a column that was not profiled, or is not in the profile", () => {
    expect(valueShares(column({ profiled: false }), 1000)).toBeNull()
    expect(valueShares(undefined, 1000)).toBeNull()
  })

  it("has no values and all of the non-null rows as 'other' for a column with nothing listed", () => {
    const shares = valueShares(column({ topValues: undefined, nullFraction: 0.25, nullCount: 250 }), 1000)
    expect(shares?.values).toEqual([])
    expect(shares?.otherShare).toBe(0.75)
    expect(shares?.nullShare).toBe(0.25)
  })

  it("keeps the empty string as a value of its own", () => {
    const shares = valueShares(column({ topValues: [{ value: "", count: 250 }] }), 1000)
    expect(shares?.values).toEqual([{ value: "", count: 250, share: 0.25 }])
  })

  it("reads the null share from the null count when the fraction is missing", () => {
    const shares = valueShares(column({ nullFraction: null, nullCount: 200 }), 1000)
    expect(shares?.nullShare).toBe(0.2)
    expect(shares?.nullCount).toBe(200)
  })

  it("does not claim a null share, or an 'other', the profile did not state", () => {
    const shares = valueShares(column({ nullFraction: null, nullCount: null }), 1000)
    expect(shares?.values).toHaveLength(2)
    expect(shares?.nullShare).toBeNull()
    expect(shares?.nullCount).toBeNull()
    expect(shares?.otherShare).toBeNull()
    expect(shares?.otherCount).toBeNull()
  })

  it("skips a value whose count is not a number, rather than drawing it", () => {
    const shares = valueShares(
      column({ topValues: [{ value: "a", count: Number.NaN }, { value: "b", count: 100 }] }),
      1000
    )
    expect(shares?.values.map((v) => v.value)).toEqual(["b"])
  })
})

describe("noValuesLabel", () => {
  // A column that lists no value: `topValues` is empty.
  const bare = (over: Partial<ColumnProfile> = {}): ColumnProfile =>
    column({ topValues: [], nullCount: 0, nullFraction: 0, distinctCount: 500, ...over })

  it("says all null when every row profiled is null", () => {
    expect(noValuesLabel(bare({ nullCount: 2000, nullFraction: 1, distinctCount: 0 }), 2000)).toBe("All null")
  })

  it("says all null from the fraction when the count is missing", () => {
    expect(noValuesLabel(bare({ nullCount: null, nullFraction: 1, distinctCount: 0 }), 2000)).toBe("All null")
  })

  it("says mostly unique where the distinct count is at least 90% of the non-null rows", () => {
    expect(noValuesLabel(bare({ distinctCount: 1800 }), 2000)).toBe("Mostly unique")
    expect(noValuesLabel(bare({ distinctCount: 2000 }), 2000)).toBe("Mostly unique")
    // The count is approximate, so it can pass the rows.
    expect(noValuesLabel(bare({ distinctCount: 2100 }), 2000)).toBe("Mostly unique")
  })

  it("says many distinct values just below that line, rather than unique", () => {
    expect(noValuesLabel(bare({ distinctCount: 1799 }), 2000)).toBe("Many distinct values")
  })

  it("says many distinct values for about 500 distinct values in 2,000 rows", () => {
    // `customer` on the raw `demo-orders-param`: the case that read "Mostly unique" with `Distinct ≈ 500` beside it.
    expect(noValuesLabel(bare({ distinctCount: 500 }), 2000)).toBe("Many distinct values")
  })

  it("counts the line against the non-null rows, not all of them", () => {
    // 450 distinct of 1,000 rows is not unique, but of the 500 that are not null it is 90%.
    expect(noValuesLabel(bare({ nullCount: 500, nullFraction: 0.5, distinctCount: 450 }), 1000)).toBe(
      "Mostly unique"
    )
    expect(noValuesLabel(bare({ nullCount: 500, nullFraction: 0.5, distinctCount: 449 }), 1000)).toBe(
      "Many distinct values"
    )
  })

  it("says many distinct values when the profile gave no distinct count", () => {
    expect(noValuesLabel(bare({ distinctCount: null }), 2000)).toBe("Many distinct values")
    expect(noValuesLabel(bare({ distinctCount: undefined }), 2000)).toBe("Many distinct values")
  })

  it("never says all null, and counts against every row, when no null count was stated", () => {
    const unknown = bare({ nullCount: null, nullFraction: null, distinctCount: 1800 })
    expect(noValuesLabel(unknown, 2000)).toBe("Mostly unique")
    expect(noValuesLabel({ ...unknown, distinctCount: 0 }, 2000)).toBe("Many distinct values")
  })

  it("has nothing to say for a column that was not profiled, is not in the profile, or had no rows", () => {
    expect(noValuesLabel(bare({ profiled: false }), 2000)).toBeNull()
    expect(noValuesLabel(undefined, 2000)).toBeNull()
    expect(noValuesLabel(bare(), 0)).toBeNull()
    expect(noValuesLabel(bare(), Number.NaN)).toBeNull()
  })
})

describe("formatShare", () => {
  it("writes one decimal, like the null share", () => {
    expect(formatShare(0.5)).toBe("50.0%")
    expect(formatShare(1)).toBe("100.0%")
    expect(formatShare(0)).toBe("0.0%")
  })

  it("does not write a share too small to show as zero", () => {
    expect(formatShare(0.0004)).toBe("<0.1%")
    // 0.056%: `formatPercent` alone would round this up to "0.1%".
    expect(formatShare(0.00056)).toBe("<0.1%")
    expect(formatShare(0.001)).toBe("0.1%")
  })
})

describe("sortKeyColumns", () => {
  const names = ["plnt", "material", "d", "a", "b", "c"]

  it("marks the one column a key of one column names", () => {
    expect([...sortKeyColumns("plnt", names)]).toEqual(["plnt"])
  })

  it("marks both columns of a key of two, with or without spaces around the comma", () => {
    expect([...sortKeyColumns("plnt, material", names)].sort()).toEqual(["material", "plnt"])
    expect([...sortKeyColumns("plnt,material", names)].sort()).toEqual(["material", "plnt"])
    expect([...sortKeyColumns("  plnt  ,   material  ", names)].sort()).toEqual(["material", "plnt"])
  })

  it("marks nothing for an expression, though the column is inside it", () => {
    expect(sortKeyColumns("toYYYYMM(d)", names).size).toBe(0)
  })

  it("marks the plain columns beside an expression, and only those", () => {
    expect([...sortKeyColumns("toYYYYMM(d), plnt", names)]).toEqual(["plnt"])
  })

  it("does not split an expression at its own commas, so its arguments mark nothing", () => {
    expect([...sortKeyColumns("cityHash64(a, b, c), material", names)]).toEqual(["material"])
  })

  it("marks nothing for a name the table does not have", () => {
    expect(sortKeyColumns("plnt, gone", ["plnt"]).has("gone")).toBe(false)
  })

  it("matches the whole name, not a part of it", () => {
    expect(sortKeyColumns("plnt_code", ["plnt"]).size).toBe(0)
    expect(sortKeyColumns("plnt", ["plnt_code"]).size).toBe(0)
  })

  it("reads a name written in backticks or double quotes as the name", () => {
    expect([...sortKeyColumns("`material`, \"plnt\"", names)].sort()).toEqual(["material", "plnt"])
  })

  it("marks nothing for an empty key, a blank one or none", () => {
    expect(sortKeyColumns("", names).size).toBe(0)
    expect(sortKeyColumns("   ", names).size).toBe(0)
    expect(sortKeyColumns(null, names).size).toBe(0)
    expect(sortKeyColumns(undefined, names).size).toBe(0)
  })
})
