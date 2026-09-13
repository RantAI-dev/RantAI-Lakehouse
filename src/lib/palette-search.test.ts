import { describe, expect, it } from "bun:test"
import { PALETTE_ASSET_RESULT_LIMIT, capPaletteAssetResults } from "./palette-search"

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
