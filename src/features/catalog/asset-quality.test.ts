// A rule added from an asset's Quality tab must be one the evaluator can
// run: the form's threshold is written in exactly the grammar
// `routes::quality::parse_threshold` reads.
import { describe, expect, it } from "bun:test"
import { ruleThreshold } from "./asset-quality"

const form = { kind: "not-null", column: "email", rows: "1", percent: "100", min: "", max: "" } as const

describe("ruleThreshold", () => {
  it("writes each kind of check in the evaluator's grammar", () => {
    expect(ruleThreshold({ ...form, kind: "rows", rows: "1000" })).toBe("rows >= 1000")
    expect(ruleThreshold(form)).toBe("email not null")
    expect(ruleThreshold({ ...form, percent: "95" })).toBe("email not null >= 95%")
    expect(ruleThreshold({ ...form, kind: "unique" })).toBe("email unique")
    expect(ruleThreshold({ ...form, kind: "range", column: "amount", min: "0", max: "100.50" })).toBe(
      "amount between 0 and 100.5"
    )
    expect(ruleThreshold({ ...form, kind: "range", column: "amount", min: "0" })).toBe("amount >= 0")
    expect(ruleThreshold({ ...form, kind: "range", column: "amount", max: "9" })).toBe("amount <= 9")
  })

  it("gives nothing while the form cannot be run", () => {
    expect(ruleThreshold({ ...form, kind: "rows", rows: "many" })).toBeNull()
    expect(ruleThreshold({ ...form, percent: "120" })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "range", column: "amount" })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "range", column: "amount", min: "low" })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "unique", column: "" })).toBeNull()
  })
})
