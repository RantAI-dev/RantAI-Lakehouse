import { describe, expect, it } from "bun:test"
import { alertRuleFormFields } from "./alerts-page"

// Pure logic, no DOM (matching src/lib/*.test.ts's convention): which form
// fields the rule editor shows for each `AlertRule.type`. `freshness`
// targets a `dataset_sla.table_name` via the reused `mart` field and sends
// no board — the backend clears measure/agg/threshold/board for this kind
// (WS5 item E3).
describe("alertRuleFormFields", () => {
  it("shows the freshness target field, not mart/measure or board, for a freshness rule", () => {
    expect(alertRuleFormFields("freshness")).toEqual({
      martMeasure: false,
      board: false,
      freshnessTarget: true,
    })
  })

  it("shows mart/measure, not board or the freshness target, for an alert rule", () => {
    expect(alertRuleFormFields("alert")).toEqual({
      martMeasure: true,
      board: false,
      freshnessTarget: false,
    })
  })

  it("shows the board picker, not mart/measure or the freshness target, for a digest rule", () => {
    expect(alertRuleFormFields("digest")).toEqual({
      martMeasure: false,
      board: true,
      freshnessTarget: false,
    })
  })
})
