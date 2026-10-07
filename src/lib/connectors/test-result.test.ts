import { describe, expect, it } from "bun:test"
import { TEST_OUTCOME_TOAST, testOutcome } from "@/lib/connectors/test-result"

describe("testOutcome", () => {
  it("calls a probe that ran and connected a pass, and one that ran and did not a failure", () => {
    expect(testOutcome({ ok: true, supported: true })).toBe("passed")
    expect(testOutcome({ ok: false, supported: true })).toBe("failed")
  })

  it("never reads a type this build cannot dial as a pass or a failure, whatever ok holds", () => {
    expect(testOutcome({ ok: false, supported: false })).toBe("unsupported")
    expect(testOutcome({ ok: true, supported: false })).toBe("unsupported")
  })

  it("has a distinct toast sentence for each outcome", () => {
    expect(TEST_OUTCOME_TOAST).toEqual({
      passed: "Connection test passed",
      failed: "Connection test failed",
      unsupported: "This connector type cannot be tested",
    })
  })
})
