import { describe, expect, it } from "bun:test"

import { APPROVAL_STATE_TEXT, approvalErrorOf, approvalStateOf } from "./approval-state"

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

  it("tells an executed-and-failed run from one that never ran (AI-28 review R1)", () => {
    // The decide route records `failed` both before and after execution;
    // only the last step's label tells the two apart.
    expect(approvalStateOf("failed", "Executed after approval")).toBe("failed")
    expect(approvalStateOf("failed", "Not executed")).toBe("not_run")
    expect(APPROVAL_STATE_TEXT.failed.line).toBe(
      "Approved; the action ran and failed."
    )
  })

  it("treats a rejected run as rejected", () => {
    expect(approvalStateOf("rejected")).toBe("rejected")
  })

  it("leaves an unknown status pending", () => {
    expect(approvalStateOf("something_new")).toBe("pending")
  })

  it("gives every state a badge and a line", () => {
    for (const state of ["pending", "ran", "not_run", "rejected", "failed"] as const) {
      expect(APPROVAL_STATE_TEXT[state].badge.length).toBeGreaterThan(0)
      expect(APPROVAL_STATE_TEXT[state].line.length).toBeGreaterThan(0)
    }
  })
})

describe("approvalErrorOf", () => {
  // AI-11: the sentence the tool returned, read from the step the
  // decide route stored after an approved call ran.
  it("returns the tool's own sentence when an executed call failed", () => {
    const detail = JSON.stringify({ error: "Only another manager of this dashboard can approve this." })
    expect(approvalErrorOf("Executed after approval", detail)).toBe(
      "Only another manager of this dashboard can approve this."
    )
  })

  it("gives no sentence for a call that never ran, or a detail without one", () => {
    expect(approvalErrorOf("Not executed", JSON.stringify({ error: "x" }))).toBeUndefined()
    expect(approvalErrorOf("Executed after approval", "not json")).toBeUndefined()
    expect(approvalErrorOf("Executed after approval", JSON.stringify({ ok: true }))).toBeUndefined()
    expect(approvalErrorOf("Executed after approval", "")).toBeUndefined()
  })
})
