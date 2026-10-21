// A rule added from an asset's Quality tab must be one the evaluator can
// run: the form's threshold is written in exactly the grammar
// `routes::quality::parse_threshold` reads.
import { describe, expect, it } from "bun:test"
import { EMPTY_FORM } from "@/features/governance/quality-checks"
import { ruleThreshold } from "./asset-quality"

const form = { ...EMPTY_FORM, column: "email" } as const

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

// DATA-14 (F1): every added check is written in the grammar the API reads
// (`routes::quality::parse_threshold`).
describe("ruleThreshold, the added checks", () => {
  it("writes each added check in the evaluator's grammar", () => {
    expect(ruleThreshold({ ...form, kind: "nulls", amount: "5" })).toBe("email nulls <= 5%")
    expect(ruleThreshold({ ...form, kind: "nulls", amount: "10", nullUnit: "count" })).toBe(
      "email nulls <= 10"
    )
    expect(ruleThreshold({ ...form, kind: "duplicates", amount: "3" })).toBe("email duplicates <= 3")
    expect(ruleThreshold({ ...form, kind: "distinct", amount: "2" })).toBe("email distinct >= 2")
    expect(ruleThreshold({ ...form, kind: "fresh", column: "loaded_at", amount: "6" })).toBe(
      "loaded_at fresh within 6h"
    )
    expect(
      ruleThreshold({ ...form, kind: "fresh", column: "loaded_at", amount: "30", durationUnit: "m" })
    ).toBe("loaded_at fresh within 30m")
    expect(ruleThreshold({ ...form, kind: "accepted", column: "status", values: "new\npaid\n" })).toBe(
      "status in ('new', 'paid')"
    )
    expect(ruleThreshold({ ...form, kind: "accepted", column: "status", values: "it's" })).toBe(
      "status in ('it''s')"
    )
    expect(ruleThreshold({ ...form, kind: "stat", column: "amount", stat: "avg", min: "10", max: "20" })).toBe(
      "avg(amount) between 10 and 20"
    )
    expect(ruleThreshold({ ...form, kind: "stat", column: "amount", stat: "min", min: "0" })).toBe(
      "min(amount) >= 0"
    )
    expect(ruleThreshold({ ...form, kind: "stat", column: "amount", stat: "max", max: "9" })).toBe(
      "max(amount) <= 9"
    )
  })

  it("gives nothing while an added check cannot be run", () => {
    expect(ruleThreshold({ ...form, kind: "nulls", amount: "120" })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "nulls", amount: "1.5", nullUnit: "count" })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "duplicates", amount: "-1" })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "fresh", amount: "0" })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "fresh", amount: "x" })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "accepted", values: "  \n" })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "accepted", values: "a".repeat(201) })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "stat", column: "amount" })).toBeNull()
    expect(ruleThreshold({ ...form, kind: "distinct", column: "", amount: "2" })).toBeNull()
  })
})
