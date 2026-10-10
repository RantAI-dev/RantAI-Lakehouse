import { describe, expect, it } from "bun:test"
import { failureStreakLabel, latestRun } from "./connectors-columns"

// SRC-7 task 9: the Sources "Last run" column and the streak beside the badge.
describe("latestRun", () => {
  it("is the later of the last success and the last failure, with which one it was", () => {
    expect(
      latestRun({ lastRunSuccessAt: "2026-10-01T02:00:00Z", lastRunFailureAt: "2026-10-02T02:00:00Z" })
    ).toEqual({ at: "2026-10-02T02:00:00Z", succeeded: false })
    expect(
      latestRun({ lastRunSuccessAt: "2026-10-03T02:00:00Z", lastRunFailureAt: "2026-10-02T02:00:00Z" })
    ).toEqual({ at: "2026-10-03T02:00:00Z", succeeded: true })
  })

  it("is the only one there is, and null when there is none (unknown, not a made-up time)", () => {
    expect(latestRun({ lastRunSuccessAt: "2026-10-01T02:00:00Z", lastRunFailureAt: null })).toEqual({
      at: "2026-10-01T02:00:00Z",
      succeeded: true,
    })
    expect(latestRun({ lastRunSuccessAt: null, lastRunFailureAt: "2026-10-01T02:00:00Z" })).toEqual({
      at: "2026-10-01T02:00:00Z",
      succeeded: false,
    })
    expect(latestRun({ lastRunSuccessAt: null, lastRunFailureAt: null })).toBeNull()
  })
})

describe("failureStreakLabel", () => {
  it("names a streak above zero and says nothing for none", () => {
    expect(failureStreakLabel(3)).toBe("3 failed in a row")
    expect(failureStreakLabel(1)).toBe("1 failed in a row")
    expect(failureStreakLabel(0)).toBeNull()
  })
})
