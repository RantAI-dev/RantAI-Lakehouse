import { afterEach, describe, expect, it, mock } from "bun:test"
import { hierarchyService } from "./hierarchies"

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

describe("a hierarchy refusal reads as the sentence that names the permission (WEB-12)", () => {
  it("a refused hierarchy list rejects with permission_denied, the status and the sentence", async () => {
    stubFetch(() => json({ error: "permission_denied: dashboard:write" }, 403))
    await expect(hierarchyService.list({ mart: "m" })).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the dashboard:write permission.",
    })
  })

  it("a 404 is still not_found with the server's sentence", async () => {
    stubFetch(() => json({ error: "No such hierarchy." }, 404))
    await expect(hierarchyService.list({ mart: "m" })).rejects.toMatchObject({
      code: "not_found",
      message: "No such hierarchy.",
    })
  })
})
