import { describe, expect, it } from "bun:test"
import { renderTransformDraft, transformErrorRowIndex } from "./transform-draft"

describe("renderTransformDraft", () => {
  it("renders dedupe as dedupe(key) — matches transform_grammar.rs's Dedupe arm", () => {
    expect(renderTransformDraft({ verb: "dedupe", key: "order_id" })).toBe("dedupe(order_id)")
  })

  it("renders filter as filter(col op 'value') — matches transform_grammar.rs's parse_filter grammar", () => {
    expect(
      renderTransformDraft({ verb: "filter", column: "status", operator: "=", value: "active" })
    ).toBe("filter(status = 'active')")
  })

  it("renders rename as rename(from,to) — matches transform_grammar.rs's Rename arm", () => {
    expect(renderTransformDraft({ verb: "rename", from: "old_col", to: "new_col" })).toBe(
      "rename(old_col,new_col)"
    )
  })

  it("renders cast as cast(col,type) — matches transform_grammar.rs's Cast arm", () => {
    expect(renderTransformDraft({ verb: "cast", column: "amount", type: "Int64" })).toBe(
      "cast(amount,Int64)"
    )
  })

  it("renders select as select(cols) — matches transform_grammar.rs's Select arm", () => {
    expect(renderTransformDraft({ verb: "select", columns: "id,name,amount" })).toBe(
      "select(id,name,amount)"
    )
  })
})

describe("transformErrorRowIndex", () => {
  it("extracts the failing row index from the server's transforms[N] message", () => {
    expect(
      transformErrorRowIndex("invalid transform at transforms[0]: unknown transform verb")
    ).toBe(0)
    expect(
      transformErrorRowIndex("invalid transform at transforms[3]: cast type not allowed")
    ).toBe(3)
  })

  it("returns null for a message that does not name a transforms[N] index", () => {
    expect(transformErrorRowIndex("database error")).toBeNull()
  })
})
