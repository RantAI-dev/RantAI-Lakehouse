import { afterEach, describe, expect, it, mock } from "bun:test"
import { clickhouseOpsService } from "./ops"

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

describe("ops refusals (WEB-12)", () => {
  it("a refused service list rejects as permission_denied, not unavailable", async () => {
    stubFetch(() => json({ error: "permission_denied: ops:read" }, 403))
    await expect(clickhouseOpsService.listServices()).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the ops:read permission.",
    })
  })
})
