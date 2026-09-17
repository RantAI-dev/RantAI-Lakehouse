import { describe, expect, it } from "bun:test"
import { OMITTED_TABLE_TEXT, tokenizeInline } from "./citation-markers"

describe("tokenizeInline", () => {
  it("passes plain prose through as a single text token", () => {
    expect(tokenizeInline("Ada 42 baris.")).toEqual([{ kind: "text", content: "Ada 42 baris." }])
  })

  it("still recognizes bold, code, and italic (pre-existing behaviour, unchanged)", () => {
    expect(tokenizeInline("a **b** c `d` e *f*")).toEqual([
      { kind: "text", content: "a " },
      { kind: "bold", content: "b" },
      { kind: "text", content: " c " },
      { kind: "code", content: "d" },
      { kind: "text", content: " e " },
      { kind: "italic", content: "f" },
    ])
  })

  it("extracts an unverified-number span's inner digits into its own token", () => {
    // The exact string citations.rs's annotate_line emits (see
    // an_unverified_number_is_flagged_inline_not_silently_removed in
    // rust/crates/lakehouse-api/src/routes/ai/citations.rs).
    const answer = 'Total pendapatan adalah <span data-unverified="true">999999</span>.'
    expect(tokenizeInline(answer)).toEqual([
      { kind: "text", content: "Total pendapatan adalah " },
      { kind: "unverified", content: "999999" },
      { kind: "text", content: "." },
    ])
  })

  it("recognizes the omitted-table literal as its own token, not as prose", () => {
    expect(tokenizeInline(OMITTED_TABLE_TEXT)).toEqual([{ kind: "omitted-table" }])
  })

  it("recognizes a verified number as plain text — no wrapper, no token", () => {
    // A verified number is left byte-for-byte unchanged by the backend
    // (citations.rs: a_verified_number_is_left_exactly_as_printed_with_no_wrapper_at_all).
    // There is no positive-state marker to tokenize.
    expect(tokenizeInline("Ada 42 baris.")).toEqual([{ kind: "text", content: "Ada 42 baris." }])
  })

  it("handles multiple markers in one line, preserving order and surrounding text", () => {
    const answer =
      'Skor: <span data-unverified="true">777</span> dari verified 42, dan bold **catatan**.'
    expect(tokenizeInline(answer)).toEqual([
      { kind: "text", content: "Skor: " },
      { kind: "unverified", content: "777" },
      { kind: "text", content: " dari verified 42, dan bold " },
      { kind: "bold", content: "catatan" },
      { kind: "text", content: "." },
    ])
  })

  it("keeps an unverified table cell's marker distinct from the surrounding row text", () => {
    // Mirrors a partially-verified table row: only the unverified cell is
    // wrapped (citations.rs: annotate_table_block).
    const row = '| 1 | <span data-unverified="true">998</span> |'
    expect(tokenizeInline(row)).toEqual([
      { kind: "text", content: "| 1 | " },
      { kind: "unverified", content: "998" },
      { kind: "text", content: " |" },
    ])
  })
})
