import { describe, expect, it } from "bun:test"
import { isValidLagLimit, lagTone, lagWords, orderedChecks } from "./capture-lag"

describe("lagWords", () => {
  it("words each status and shows a number only for `behind`", () => {
    expect(lagWords({ status: "up_to_date" })).toBe("Up to date")
    expect(lagWords({ status: "behind", seconds: 125 })).toBe("Behind by 2m")
    expect(lagWords({ status: "behind", seconds: 20 })).toBe("Behind by 20 s")
    expect(lagWords({ status: "not_known_yet" })).toBe("Not known yet")
    expect(lagWords({ status: "paused" })).toBe("Paused")
  })

  it("says since when for a stale table and never invents a number", () => {
    expect(lagWords({ status: "not_known_since", since: "2026-10-30T10:00:00Z" })).toMatch(/^Not known since /)
    expect(lagWords({ status: "not_known_since" })).toBe("Not known")
    expect(lagWords({ status: "behind" })).toBe("Behind")
  })

  it("shows a status this build does not know as it was sent, never as up to date", () => {
    expect(lagWords({ status: "something_new" })).toBe("something_new")
  })
})

describe("lagTone", () => {
  it("is green up to date and amber past the limit or when stale", () => {
    expect(lagTone({ status: "up_to_date" }, 15)).toBe("success")
    expect(lagTone({ status: "behind" }, 15, 16 * 60)).toBe("warning")
    expect(lagTone({ status: "behind" }, 15, 15 * 60)).toBe("neutral")
    expect(lagTone({ status: "not_known_since" }, 15)).toBe("warning")
    expect(lagTone({ status: "paused" }, 15)).toBe("neutral")
  })
})

describe("isValidLagLimit", () => {
  it("takes a whole number of minutes from 1 to 1440 and nothing else", () => {
    for (const ok of ["1", "15", "1440", " 30 "]) expect(isValidLagLimit(ok)).toBe(true)
    for (const bad of ["", "0", "1441", "1.5", "-3", "abc", "1e2"]) expect(isValidLagLimit(bad)).toBe(false)
  })
})

describe("orderedChecks", () => {
  it("puts the missing checks first, then the unread, then the passing, keeping the order within each", () => {
    const check = (setting: string, status: string) => ({ setting, status, message: "m" })
    const ordered = orderedChecks([
      check("a", "ok"),
      check("b", "missing"),
      check("c", "unchecked"),
      check("d", "ok"),
      check("e", "missing"),
    ])
    expect(ordered.map((c) => c.setting)).toEqual(["b", "e", "c", "a", "d"])
  })
})
