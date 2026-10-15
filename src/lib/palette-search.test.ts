import { describe, expect, it } from "bun:test"
import { PALETTE_ASSET_RESULT_LIMIT, capPaletteAssetResults, matchedOnLabel } from "./palette-search"

describe("capPaletteAssetResults", () => {
  it("passes through a result set at or under the limit unchanged", () => {
    const results = ["a", "b", "c"]
    expect(capPaletteAssetResults(results)).toEqual(["a", "b", "c"])
  })

  it("truncates to the limit, preserving order", () => {
    const results = Array.from({ length: 20 }, (_, i) => i)
    const capped = capPaletteAssetResults(results)
    expect(capped.length).toBe(PALETTE_ASSET_RESULT_LIMIT)
    expect(capped).toEqual([0, 1, 2, 3, 4, 5, 6, 7])
  })

  it("returns an empty array for an empty result set", () => {
    expect(capPaletteAssetResults([])).toEqual([])
  })
})

describe("matchedOnLabel", () => {
  it("names the column or the tag that matched", () => {
    expect(matchedOnLabel({ field: "column", value: "revenue_amount", approximate: false })).toBe(
      "column revenue_amount",
    )
    expect(matchedOnLabel({ field: "tag", value: "finance", approximate: false })).toBe("tag finance")
  })

  it("names a field that carries no value by its own name", () => {
    expect(matchedOnLabel({ field: "description", value: "", approximate: false })).toBe("description")
    expect(matchedOnLabel({ field: "owner", value: "", approximate: false })).toBe("owner")
    expect(matchedOnLabel({ field: "columnDescription", value: "", approximate: false })).toBe(
      "column description",
    )
  })

  it("says so when a typo was forgiven", () => {
    expect(matchedOnLabel({ field: "column", value: "revenue", approximate: true })).toBe(
      "column revenue · approximate match",
    )
  })

  it("has nothing to say when the server sent no reason", () => {
    expect(matchedOnLabel(undefined)).toBeNull()
  })
})
