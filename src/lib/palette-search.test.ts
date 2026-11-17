import { describe, expect, it } from "bun:test"
import { readFileSync } from "node:fs"
import {
  PALETTE_ASSET_RESULT_LIMIT,
  PALETTE_SEARCH_MAX_CHARS,
  capPaletteAssetResults,
  makePaletteFilter,
  matchedOnLabel,
  paletteAssetLines,
} from "./palette-search"

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

describe("PALETTE_SEARCH_MAX_CHARS", () => {
  // QA `LH-DATA-11-15` F2 (`DATA-54`): the box and the dashboard search must
  // refuse the same length. The Rust file is read, the way
  // `capabilities.test.ts` reads its fixture.
  it("the box's limit is the dashboard search's", () => {
    const rust = readFileSync(
      `${process.cwd()}/rust/crates/lakehouse-bi/src/search.rs`,
      "utf8",
    )
    expect(rust).toContain(`pub const MAX_QUERY_CHARS: usize = ${PALETTE_SEARCH_MAX_CHARS};`)
  })
})

describe("makePaletteFilter", () => {
  // QA `LH-DATA-11-14` (`DATA-54`): cmdk accepts the letters in order
  // anywhere, so `gic` was found in "Governance Access rules
  // /governance/policies".
  it("a page is listed only when every typed word is in its text", () => {
    const filter = makePaletteFilter([])
    const page = "Governance Access rules /governance/policies"
    expect(filter(page, "gic")).toBe(0)
    expect(filter(page, "polic")).toBeGreaterThan(0)
    expect(filter(page, "access")).toBeGreaterThan(0)
    expect(filter(page, "ACCESS rules")).toBeGreaterThan(0)
    expect(filter("theme dark light", "dark")).toBeGreaterThan(0)
    expect(filter("theme dark light", "drk")).toBe(0)
  })
})

describe("paletteAssetLines", () => {
  // QA `LH-DATA-11-01` F1 (`DATA-54`): the result shows the table's physical
  // name, and the reason; a table-key match is not printed twice.
  it("the lines of a result are its table key and why it matched", () => {
    expect(
      paletteAssetLines({
        id: "raw-orders",
        tableKey: "bronze.orders",
        matchedOn: { field: "column", value: "customer_id", approximate: false },
      }),
    ).toEqual({ path: "bronze.orders", reason: "column customer_id" })
    expect(
      paletteAssetLines({
        id: "silver.dim_negara",
        matchedOn: { field: "table", value: "silver.dim_negara", approximate: false },
      }),
    ).toEqual({ path: "silver.dim_negara", reason: null })
    expect(
      paletteAssetLines({
        id: "silver.dim_negara",
        matchedOn: { field: "table", value: "silver.dim_negara", approximate: true },
      }),
    ).toEqual({ path: "silver.dim_negara", reason: "approximate match" })
    // A Bronze row whose registry records no table name has no key.
    expect(
      paletteAssetLines({
        id: "raw-orders",
        matchedOn: { field: "tag", value: "pii", approximate: false },
      }),
    ).toEqual({ path: null, reason: "tag pii" })
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

  // QA `LH-DATA-11-05`, `LH-DATA-11-14` (`DATA-54`): the server names the
  // table it read, which is the physical name, not an internal id.
  it("names the table that matched, and calls an old server's id a table id", () => {
    expect(
      matchedOnLabel({ field: "table", value: "silver.dim_negara", approximate: false }),
    ).toBe("table silver.dim_negara")
    expect(matchedOnLabel({ field: "table", value: "silver.dim_negara", approximate: true })).toBe(
      "table silver.dim_negara · approximate match",
    )
    expect(matchedOnLabel({ field: "id", value: "", approximate: true })).toBe(
      "table id · approximate match",
    )
  })

  // QA `LH-DATA-11-04` (`DATA-54`): a description match quotes the piece the
  // server sent.
  it("quotes the piece of the description the server sent", () => {
    expect(
      matchedOnLabel({ field: "description", value: "monthly revenue by region", approximate: false }),
    ).toBe('description: "monthly revenue by region"')
  })

  it("says so when a typo was forgiven", () => {
    expect(matchedOnLabel({ field: "column", value: "revenue", approximate: true })).toBe(
      "column revenue · approximate match",
    )
  })

  it("says only that a name match was approximate, the name being on screen (DATA-32)", () => {
    expect(matchedOnLabel({ field: "name", value: "", approximate: true })).toBe(
      "approximate match",
    )
  })

  it("has nothing to say when the server sent no reason", () => {
    expect(matchedOnLabel(undefined)).toBeNull()
  })
})
