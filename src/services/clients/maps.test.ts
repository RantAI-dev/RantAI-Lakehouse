import { afterEach, describe, expect, it, mock } from "bun:test"
import { mapService } from "./maps"

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

describe("a map refusal reads as the sentence that names the permission (WEB-12)", () => {
  it("a refused map list rejects with permission_denied, the status and the sentence", async () => {
    stubFetch(() => json({ error: "permission_denied: dashboard:write" }, 403))
    await expect(mapService.list()).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the dashboard:write permission.",
    })
  })

  it("a refused map delete rejects with permission_denied, and a map in use is still an answer", async () => {
    stubFetch(() => json({ error: "permission_denied: dashboard:write" }, 403))
    await expect(mapService.remove("m1")).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the dashboard:write permission.",
    })
    stubFetch(() => json({ error: "This map is used by charts.", charts: [] }, 409))
    expect(await mapService.remove("m1")).toEqual({
      ok: false,
      message: "This map is used by charts.",
      charts: [],
    })
  })
})
