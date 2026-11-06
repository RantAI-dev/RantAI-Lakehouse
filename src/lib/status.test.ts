import { describe, expect, it } from "bun:test"
import { ENTITY_STATUS_DESCRIPTION, ENTITY_STATUS_LABEL, isEmployeePaused } from "./status"

describe("isEmployeePaused", () => {
  it("reads a suspended employee's status as paused", () => {
    expect(isEmployeePaused("paused")).toBe(true)
  })

  it("does not read a revoked employee's status as paused (resume would 409)", () => {
    expect(isEmployeePaused("cancelled")).toBe(false)
  })

  it("does not read a running employee's status as paused", () => {
    expect(isEmployeePaused("running")).toBe(false)
  })
})

describe("the stopped status", () => {
  it("has its own label and description, apart from cancelled", () => {
    expect(ENTITY_STATUS_LABEL.stopped).toBe("Stopped")
    expect(ENTITY_STATUS_DESCRIPTION.stopped).toBe("Stopped by the person who ran it.")
    expect(ENTITY_STATUS_LABEL.cancelled).toBe("Cancelled")
  })
})
