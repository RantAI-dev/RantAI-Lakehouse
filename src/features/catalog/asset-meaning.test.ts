import { describe, expect, it } from "bun:test"
import type { SemanticEntry, SemanticEntryInput } from "@/services/contracts/semantic"
import { draftCount, inputOf, parseSynonyms, roleLabel, semanticAsset, splitEntries } from "./asset-meaning"
import { visibleColumns } from "./asset-columns"

const entry = (over: Partial<SemanticEntry> = {}): SemanticEntry => ({
  asset: "serving.orders",
  column: null,
  description: "",
  synonyms: [],
  role: null,
  status: "confirmed",
  writtenBy: null,
  model: null,
  updatedAt: "2026-10-01T00:00:00Z",
  ...over,
})

describe("semanticAsset", () => {
  it("returns the key for serving and silver tables and null for any other key", () => {
    expect(semanticAsset("serving.a")).toBe("serving.a")
    expect(semanticAsset("silver.b")).toBe("silver.b")
    expect(semanticAsset("bronze.c")).toBeNull()
    expect(semanticAsset("orders")).toBeNull()
  })
})

describe("splitEntries", () => {
  it("returns the table entry first and keys the column entries by their column name", () => {
    const table = entry({ column: null })
    const { table: found, byColumn } = splitEntries([
      table,
      entry({ column: "omzet" }),
      entry({ column: "tanggal" }),
    ])
    expect(found).toBe(table)
    expect([...byColumn.keys()].sort()).toEqual(["omzet", "tanggal"])
  })

  it("returns a null table when the entries hold column entries only", () => {
    const { table, byColumn } = splitEntries([entry({ column: "omzet" })])
    expect(table).toBeNull()
    expect(byColumn.size).toBe(1)
  })
})

describe("draftCount", () => {
  it("counts only draft entries of columns that are among the rows", () => {
    const rows = [
      { column: { name: "omzet" } },
      { column: { name: "tanggal" } },
      { column: { name: "wilayah" } },
    ]
    const byColumn = new Map<string, SemanticEntry>([
      ["omzet", entry({ column: "omzet", status: "draft" })],
      ["tanggal", entry({ column: "tanggal", status: "confirmed" })],
      ["gone", entry({ column: "gone", status: "draft" })],
    ])
    expect(draftCount(rows, byColumn)).toBe(1)
  })
})

describe("parseSynonyms", () => {
  it("splits on commas, trims, drops empty pieces and repeats", () => {
    expect(parseSynonyms(" omzet, penjualan ,,omzet ")).toEqual({ synonyms: ["omzet", "penjualan"] })
  })

  it("returns the at-most-six error for seven names", () => {
    expect(parseSynonyms("a,b,c,d,e,f,g")).toEqual({ error: "At most 6 names." })
  })

  it("accepts a name of 21 astral characters, which is 21 code points though 42 UTF-16 units", () => {
    // U+1D400 (mathematical bold capital A) is outside the BMP: two UTF-16 units, one code point.
    const astral = "\u{1D400}".repeat(21)
    expect(astral.length).toBe(42)
    expect(parseSynonyms(astral)).toEqual({ synonyms: [astral] })
  })

  it("returns an error that names a piece longer than 40 characters", () => {
    const long = "x".repeat(41)
    const result = parseSynonyms(`omzet,${long}`)
    expect("error" in result).toBe(true)
    if ("error" in result) expect(result.error).toContain(long)
  })
})

describe("inputOf", () => {
  it("carries the entry's column, description and synonyms, with the overridden role", () => {
    const source = entry({
      column: "omzet",
      description: "Total sales",
      synonyms: ["revenue", "penjualan"],
      role: "measure",
    })
    const input: SemanticEntryInput = inputOf(source, { role: "non_additive" })
    expect(input).toEqual({
      column: "omzet",
      description: "Total sales",
      synonyms: ["revenue", "penjualan"],
      role: "non_additive",
    })
  })
})

describe("roleLabel", () => {
  it("returns Not set for a null role and the role's own text for an unlisted role", () => {
    expect(roleLabel(null)).toBe("Not set")
    expect(roleLabel("measure")).toBe("Measure")
    expect(roleLabel("geo")).toBe("geo")
  })

  it("returns the short label of a role that has one, and the full label otherwise", () => {
    expect(roleLabel("non_additive", "short")).toBe("Not additive")
    expect(roleLabel("non_additive")).toBe("Number, not to be added up")
    expect(roleLabel("measure", "short")).toBe("Measure")
    expect(roleLabel("geo", "short")).toBe("geo")
  })
})

describe("visibleColumns with an extra search text", () => {
  it("matches a row by the extra text when its name and description do not hold the query", () => {
    const row = {
      column: { name: "amount", dataType: "Decimal", description: "Gross figure" },
      hint: "omzet",
    }
    const other = { column: { name: "wilayah", dataType: "String" }, hint: "" }
    const { shown, matched } = visibleColumns([row, other], "omzet", 10, (r) => r.hint)
    expect(shown).toEqual([row])
    expect(matched).toBe(1)
  })
})
