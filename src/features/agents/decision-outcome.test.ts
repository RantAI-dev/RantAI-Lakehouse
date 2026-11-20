import { describe, expect, it } from "bun:test"

import { decisionOutcomeText } from "./decision-outcome"

describe("decisionOutcomeText", () => {
  it("says the action ran when the tool executed", () => {
    expect(decisionOutcomeText({ executed: true }, "approved")).toBe(
      "Approved and run."
    )
  })

  it("names the missing permission when a recorded decision did not run (AI-28, LH-AI-18-07)", () => {
    expect(
      decisionOutcomeText(
        { executed: false, notExecuted: { reason: "permission", required: "dashboard:write" } },
        "approved"
      )
    ).toBe(
      "Approved, but not run: you do not have the permission this action needs (dashboard:write)."
    )
  })

  it("says a reject ran nothing", () => {
    expect(decisionOutcomeText({ executed: false }, "rejected")).toBe(
      "Rejected. Nothing ran."
    )
  })

  it("falls back to a recorded decision for any other non-execution", () => {
    expect(decisionOutcomeText({ executed: false }, "approved")).toBe(
      "Decision recorded. Nothing ran."
    )
  })
})
