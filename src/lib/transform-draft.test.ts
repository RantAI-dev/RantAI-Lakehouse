import { describe, expect, it } from "bun:test"
import {
  TRANSFORM_VERBS,
  draftIsComplete,
  emptyDraftFor,
  renderTransformDraft,
} from "./transform-draft"

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

// BLD-1 part 1 S3-T2: the two helpers the editor's operation rows use moved
// here from the component, so no rule lives in a component.
describe("emptyDraftFor and draftIsComplete", () => {
  it("starts each verb blank, with the first operator and cast type", () => {
    expect(emptyDraftFor("filter")).toEqual({ verb: "filter", column: "", operator: "=", value: "" })
    expect(emptyDraftFor("cast")).toEqual({ verb: "cast", column: "", type: "String" })
    expect(TRANSFORM_VERBS.map((verb) => emptyDraftFor(verb).verb)).toEqual(TRANSFORM_VERBS)
  })

  it("lets a draft be added only once its own required fields are filled", () => {
    expect(draftIsComplete(emptyDraftFor("dedupe"))).toBe(false)
    expect(draftIsComplete({ verb: "dedupe", key: "id" })).toBe(true)
    expect(draftIsComplete({ verb: "filter", column: "status", operator: "=", value: " " })).toBe(false)
    expect(draftIsComplete({ verb: "rename", from: "a", to: "" })).toBe(false)
    expect(draftIsComplete({ verb: "cast", column: "amount", type: "Float64" })).toBe(true)
    expect(draftIsComplete({ verb: "select", columns: "id,name" })).toBe(true)
  })
})
