// Rewriting a rule sends what the form says, trimmed — and nothing at all
// while the form is incomplete or says what the rule already does.
import { describe, expect, it } from "bun:test"
import { ruleRewrite } from "./quality-rule-edit-dialog"

const rule = { id: "q1", name: "email_complete", asset: "serving.mart_customer_segment", threshold: ">= 95%", severity: "high" }

describe("ruleRewrite", () => {
  it("is nothing to save while the rule reads as it did", () => {
    expect(ruleRewrite(rule, { asset: rule.asset, threshold: " >= 95% ", severity: "high" })).toBeNull()
  })

  it("needs a table and a threshold", () => {
    expect(ruleRewrite(rule, { asset: " ", threshold: "email not null", severity: "high" })).toBeNull()
    expect(ruleRewrite(rule, { asset: "silver.customers", threshold: "", severity: "high" })).toBeNull()
  })

  it("carries each change, trimmed", () => {
    expect(ruleRewrite(rule, { asset: " silver.customers ", threshold: "email not null >= 95%", severity: "high" })).toEqual({
      asset: "silver.customers",
      threshold: "email not null >= 95%",
      severity: "high",
    })
    // The severity alone is a change too.
    expect(ruleRewrite(rule, { asset: rule.asset, threshold: rule.threshold, severity: "low" })).toEqual({
      asset: rule.asset,
      threshold: rule.threshold,
      severity: "low",
    })
  })
})
