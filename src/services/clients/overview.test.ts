import { afterEach, describe, expect, it, mock } from "bun:test"
import { clickhouseOverviewService } from "./overview"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
})

type Call = { url: string; init: RequestInit }

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

function stubFetch(answer: (call: Call) => Response): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call: Call = { url: String(input), init: init ?? {} }
    calls.push(call)
    return answer(call)
  }) as unknown as typeof fetch
  return calls
}

describe("overview refusals (WEB-12)", () => {
  it("getSummary rejects a refusal as permission_denied, not unavailable", async () => {
    stubFetch(() => json({ error: "permission_denied: ops:read" }, 403))
    await expect(clickhouseOverviewService.getSummary()).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the ops:read permission.",
    })
  })

  it("listAlerts rejects a refusal as permission_denied, not unavailable", async () => {
    stubFetch(() => json({ error: "permission_denied: ops:read" }, 403))
    await expect(clickhouseOverviewService.listAlerts()).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the ops:read permission.",
    })
  })

  it("acknowledging an alert that is refused rejects as permission_denied, not invalid_request", async () => {
    stubFetch(() => json({ error: "permission_denied: ops:write" }, 403))
    await expect(clickhouseOverviewService.acknowledgeAlert("a1")).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the ops:write permission.",
    })
  })

  it("getSummary still calls a 503 unavailable with the server's sentence", async () => {
    stubFetch(() => json({ error: "ClickHouse is down." }, 503))
    await expect(clickhouseOverviewService.getSummary()).rejects.toMatchObject({
      code: "unavailable",
      message: "ClickHouse is down.",
    })
  })
})
