import { describe, expect, it } from "bun:test"
import { formatLifetime, relativeTime, signSnippet, suggestedLifetimeSeconds, tokenRules } from "./share-embed"

describe("formatLifetime", () => {
  it("uses the largest whole unit", () => {
    expect(formatLifetime(86_400)).toBe("24 hours")
    expect(formatLifetime(3600)).toBe("1 hour")
    expect(formatLifetime(5400)).toBe("90 minutes")
    expect(formatLifetime(60)).toBe("1 minute")
    expect(formatLifetime(45)).toBe("45 seconds")
  })
})

describe("signSnippet", () => {
  const snippet = signSnippet({ board: "b_1", origin: "https://lake.example", maxLifetimeSeconds: 7200 })

  it("signs iat, exp and a jti", () => {
    expect(snippet).toContain("iat,")
    expect(snippet).toContain("exp: iat + 600")
    expect(snippet).toContain("jti: randomUUID()")
    expect(snippet).toContain('resource: { dashboard: "b_1" }')
    expect(snippet).toContain('"https://lake.example/embed/signed/"')
  })

  it("states the limit it was given, not a built-in one", () => {
    expect(snippet).toContain("at most 7200 (2 hours)")
    expect(snippet).not.toContain("24")
  })

  it("never suggests more than the limit", () => {
    expect(suggestedLifetimeSeconds(120)).toBe(120)
    expect(signSnippet({ board: "b", origin: "o", maxLifetimeSeconds: 120 })).toContain("exp: iat + 120")
  })
})

describe("tokenRules", () => {
  it("names the limit the server reported", () => {
    expect(tokenRules(86_400)).toBe("Tokens need iat and exp and may live at most 24 hours.")
    expect(tokenRules(900)).toContain("at most 15 minutes")
  })
})

describe("relativeTime", () => {
  const now = 1_800_000_000_000
  it("reads like a person would say it", () => {
    expect(relativeTime(1_800_000_000 - 10, now)).toBe("just now")
    expect(relativeTime(1_800_000_000 - 60, now)).toBe("1 minute ago")
    expect(relativeTime(1_800_000_000 - 3 * 3600, now)).toBe("3 hours ago")
    expect(relativeTime(1_800_000_000 - 2 * 86_400, now)).toBe("2 days ago")
    expect(relativeTime(1_800_000_000 + 500, now)).toBe("just now")
  })
})
