import { describe, expect, it } from "bun:test"

import { APPROVAL_STATE_TEXT, approvalStateOf } from "./approval-state"

describe("approvalStateOf", () => {
  it("treats a waiting or running run as pending", () => {
    expect(approvalStateOf("waiting_approval")).toBe("pending")
    expect(approvalStateOf("running")).toBe("pending")
  })

  it("treats a succeeded run as ran", () => {
    expect(approvalStateOf("succeeded")).toBe("ran")
  })

  it("treats a failed or budget-exhausted run as not run", () => {
    expect(approvalStateOf("failed")).toBe("not_run")
    expect(approvalStateOf("budget_exhausted")).toBe("not_run")
  })

  it("treats a rejected run as rejected", () => {
    expect(approvalStateOf("rejected")).toBe("rejected")
  })

  it("leaves an unknown status pending", () => {
    expect(approvalStateOf("something_new")).toBe("pending")
  })

  it("gives every state a badge and a line", () => {
    for (const state of ["pending", "ran", "not_run", "rejected"] as const) {
      expect(APPROVAL_STATE_TEXT[state].badge.length).toBeGreaterThan(0)
      expect(APPROVAL_STATE_TEXT[state].line.length).toBeGreaterThan(0)
    }
  })
})
