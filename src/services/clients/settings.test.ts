import { afterEach, describe, expect, it, mock } from "bun:test"
import { settingsService } from "./settings"

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

describe("settings refusals (WEB-12)", () => {
  it("a refused settings read rejects with permission_denied, the status and the sentence", async () => {
    stubFetch(() => json({ error: "permission_denied: settings:write" }, 403))
    await expect(settingsService.getReporting()).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the settings:write permission.",
    })
  })

  it("a refused save keeps the server's sentence as invalid_request with its status", async () => {
    stubFetch(() => json({ error: "Unknown time zone." }, 400))
    await expect(settingsService.saveReporting({} as never)).rejects.toMatchObject({
      code: "invalid_request",
      status: 400,
      message: "Unknown time zone.",
    })
  })
})
