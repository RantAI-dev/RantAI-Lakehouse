import { afterEach, describe, expect, it, mock } from "bun:test"
import { reportService } from "./reports"

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

describe("a subscription refusal reads as the sentence that names the permission (WEB-12)", () => {
  it("a refused subscription list rejects with permission_denied, the status and the sentence", async () => {
    stubFetch(() => json({ error: "permission_denied: dashboard:write" }, 403))
    await expect(reportService.list("b1")).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the dashboard:write permission.",
    })
  })

  it("a refusal of another kind keeps the server's sentence as it was sent", async () => {
    stubFetch(() => json({ error: "The schedule is not valid." }, 400))
    await expect(reportService.list("b1")).rejects.toMatchObject({
      code: "invalid_request",
      status: 400,
      message: "The schedule is not valid.",
    })
  })
})
