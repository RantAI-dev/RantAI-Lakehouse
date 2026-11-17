import { describe, expect, it } from "bun:test"
import { MINUTES_SENTENCE, dataFromLabel, draftOf, draftProblem, ruleLabel, ruleOf } from "./result-cache"

describe("the freshness rule (BI-4)", () => {
  it("refuses minutes outside 1 to 1440 with the server's sentence and accepts the ends", () => {
    for (const bad of ["", "0", "1441", "2.5", "abc", "-3"]) expect(draftProblem({ mode: "minutes", minutes: bad })).toBe(MINUTES_SENTENCE)
    for (const ok of ["1", "30", "1440"]) expect(draftProblem({ mode: "minutes", minutes: ok })).toBeNull()
    expect(draftProblem({ mode: "fresh", minutes: "" })).toBeNull()
  })

  it("round-trips a rule through a draft", () => {
    expect(ruleOf(draftOf({ mode: "minutes", minutes: 45 }))).toEqual({ mode: "minutes", minutes: 45 })
    expect(ruleOf(draftOf({ mode: "fresh" }))).toEqual({ mode: "fresh" })
    expect(draftOf(null)).toEqual({ mode: "default", minutes: "" })
    expect(draftOf({ mode: "bogus" })).toEqual({ mode: "default", minutes: "" })
  })

  it("names a rule in words", () => {
    expect(ruleLabel(undefined)).toBe("Follow the default")
    expect(ruleLabel({ mode: "fresh" })).toBe("Always fresh")
    expect(ruleLabel({ mode: "minutes", minutes: 1 })).toBe("Remember for 1 minute")
    expect(ruleLabel({ mode: "minutes", minutes: 90 })).toBe("Remember for 90 minutes")
  })
})

describe("dataFromLabel", () => {
  it("writes the time in the report time zone and says when the result was remembered", () => {
    expect(dataFromLabel("2026-10-24T03:42:10Z", "Asia/Jakarta", false)).toBe("Data from 10:42")
    expect(dataFromLabel("2026-10-24T03:42:10Z", "Asia/Jakarta", true)).toBe("Data from 10:42 · remembered")
    expect(dataFromLabel("2026-10-24T03:42:10Z", "UTC", false)).toBe("Data from 03:42")
  })

  it("says UTC when the zone is missing or unknown, and nothing when there is no time", () => {
    expect(dataFromLabel("2026-10-24T03:42:10Z", undefined, false)).toBe("Data from 03:42 UTC")
    expect(dataFromLabel("2026-10-24T03:42:10Z", "Not/AZone", false)).toBe("Data from 03:42 UTC")
    expect(dataFromLabel(undefined, "UTC", true)).toBeNull()
    expect(dataFromLabel("nonsense", "UTC", true)).toBeNull()
  })
})
