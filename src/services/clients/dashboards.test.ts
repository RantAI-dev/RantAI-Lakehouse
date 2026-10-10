import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { clickhouseDashboardService } from "./dashboards"

const originalFetch = global.fetch

beforeEach(() => {
  localStorage.clear()
})

afterEach(() => {
  global.fetch = originalFetch
  localStorage.clear()
})

type Call = { url: string; init: RequestInit }

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

/** Answers every fetch with `answer` and records what was asked, in order. */
function stubFetch(answer: () => Response): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(input), init: init ?? {} })
    return answer()
  }) as unknown as typeof fetch
  return calls
}

const sent = (call: Call) => JSON.parse(String(call.init.body))

describe("embed state (SEC-12)", () => {
  it("getEmbedInfo reads the board's state from embed-info", async () => {
    const calls = stubFetch(() =>
      json({
        enabled: true,
        supported: true,
        sampleToken: "a.b.c",
        maxLifetimeSeconds: 3600,
        revokedBefore: 1_700_000_000,
        allowedOrigins: ["https://app.example.com"],
      }),
    )
    const info = await clickhouseDashboardService.getEmbedInfo("b 1")
    expect(calls[0].url).toBe("/api/dashboard/embed-info?board=b%201")
    expect(info).toEqual({
      enabled: true,
      supported: true,
      reason: undefined,
      sampleToken: "a.b.c",
      maxLifetimeSeconds: 3600,
      revokedBefore: 1_700_000_000,
      allowedOrigins: ["https://app.example.com"],
    })
  })

  it("getEmbedInfo reports an unconfigured server with its reason and no sample token", async () => {
    stubFetch(() =>
      json({ enabled: false, supported: false, reason: "embedding is not configured", maxLifetimeSeconds: 86400, revokedBefore: null, allowedOrigins: [] }),
    )
    const info = await clickhouseDashboardService.getEmbedInfo("b_1")
    expect(info.supported).toBe(false)
    expect(info.reason).toBe("embedding is not configured")
    expect(info.sampleToken).toBeUndefined()
  })

  it("setEmbedOrigins puts the list on the board and returns what the server stored", async () => {
    const calls = stubFetch(() => json({ ok: true, embedOrigins: ["https://app.example.com"] }))
    const stored = await clickhouseDashboardService.setEmbedOrigins("b_1", ["https://App.example.com"])
    expect(calls[0].url).toBe("/api/dashboard/boards")
    expect(calls[0].init.method).toBe("PUT")
    expect(sent(calls[0])).toEqual({ id: "b_1", embedOrigins: ["https://App.example.com"] })
    expect(stored).toEqual(["https://app.example.com"])
  })

  it("setEmbedOrigins carries the server's refusal as the error message", async () => {
    stubFetch(() => json({ error: '"*" is not an allowed site: wildcards are not accepted; list each site' }, 400))
    const err = await clickhouseDashboardService.setEmbedOrigins("b_1", ["*"]).catch((e: unknown) => e)
    expect(err).toBeInstanceOf(ServiceError)
    expect((err as ServiceError).code).toBe("invalid_request")
    expect((err as ServiceError).message).toContain("wildcards are not accepted")
  })

  it("revokeAllEmbedTokens asks for the withdrawal and returns the stored instant", async () => {
    const calls = stubFetch(() => json({ ok: true, embedRevokedBefore: 1_700_000_123 }))
    const at = await clickhouseDashboardService.revokeAllEmbedTokens("b_1")
    expect(sent(calls[0])).toEqual({ id: "b_1", embedRevokeAll: true })
    expect(at).toBe(1_700_000_123)
  })

  it("revokeAllEmbedTokens without the permission is permission_denied", async () => {
    stubFetch(() => json({ error: "permission_denied: dashboard:write" }, 403))
    const err = await clickhouseDashboardService.revokeAllEmbedTokens("b_1").catch((e: unknown) => e)
    expect((err as ServiceError).code).toBe("permission_denied")
  })

  it("revokeEmbedToken sends the whole token, never a bare id, and returns the withdrawn jti", async () => {
    const calls = stubFetch(() => json({ ok: true, withdrawn: "j-1" }))
    const jti = await clickhouseDashboardService.revokeEmbedToken("b_1", "a.b.c")
    expect(calls[0].url).toBe("/api/dashboard/embed-revoke")
    expect(calls[0].init.method).toBe("POST")
    expect(sent(calls[0])).toEqual({ board: "b_1", token: "a.b.c" })
    expect(jti).toBe("j-1")
  })

  it("revokeEmbedToken says so when the token has no id", async () => {
    stubFetch(() => json({ error: "that token has no id (jti), so it cannot be withdrawn on its own; withdraw all the dashboard's embed tokens instead." }, 400))
    const err = await clickhouseDashboardService.revokeEmbedToken("b_1", "a.b.c").catch((e: unknown) => e)
    expect((err as ServiceError).message).toContain("no id (jti)")
  })
})
