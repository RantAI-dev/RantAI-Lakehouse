import { describe, expect, it } from "bun:test"
import { deriveLastUpdatedMs, freshnessStatus } from "./freshness-strip"

describe("deriveLastUpdatedMs", () => {
  it("takes the max snapshot timestamp", () => {
    const detail = { snapshots: [{ timestampMs: 100 }, { timestampMs: 300 }, { timestampMs: 200 }] }
    expect(deriveLastUpdatedMs(detail)).toBe(300)
  })

  it("returns null for a table with no snapshots yet, never a fabricated zero", () => {
    expect(deriveLastUpdatedMs({ snapshots: [] })).toBeNull()
  })
})

describe("freshnessStatus", () => {
  const now = 1_000_000

  it("is late once the update is older than the expected interval", () => {
    expect(freshnessStatus(now, now - 61 * 60_000, 60)).toBe("late")
  })

  it("is ok within the expected interval", () => {
    expect(freshnessStatus(now, now - 10 * 60_000, 60)).toBe("ok")
  })

  it("is unknown (not 'ok') when the last-updated time cannot be determined", () => {
    expect(freshnessStatus(now, null, 60)).toBe("unknown")
  })
})
