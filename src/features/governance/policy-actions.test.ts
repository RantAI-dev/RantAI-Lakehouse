// A confirmation says what the action changes. Taking an enforced policy
// away lifts its masking; removing a draft lifts nothing, and must not
// claim otherwise.
import { describe, expect, it } from "bun:test"
import { policyConfirmation } from "./policy-actions"

describe("policyConfirmation", () => {
  it("warns that deleting an enforced policy lifts its masking", () => {
    const c = policyConfirmation("delete", { name: "mask-email", status: "ready" })
    expect(c.impact).toContain("It is being enforced")
    expect(c.impact).toContain("unmasked and unfiltered")
    expect(c.destructive).toBe(true)
  })

  it("says deleting a draft changes nothing that is enforced", () => {
    const c = policyConfirmation("delete", { name: "mask-email", status: "draft" })
    expect(c.impact).toBe("It is a draft, so nothing that is enforced changes.")
  })

  it("treats enforcing as a change, not as a destructive one", () => {
    expect(policyConfirmation("enforce", { name: "mask-email", status: "draft" }).destructive).toBe(false)
    expect(policyConfirmation("stop", { name: "mask-email", status: "ready" }).destructive).toBe(true)
  })
})
