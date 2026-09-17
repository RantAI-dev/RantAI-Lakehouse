import { describe, expect, it } from "bun:test"
import { isEmployeePaused } from "./status"

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
