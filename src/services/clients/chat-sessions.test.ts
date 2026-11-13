import { afterEach, describe, expect, it, mock } from "bun:test"
import { chatSessionService } from "./chat-sessions"

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

describe("conversation refusals (WEB-12 D4)", () => {
  it("a refused conversation list names the permissions any one of which would do", async () => {
    stubFetch(() => json({ error: "permission_denied: assistant:use, assistant:ask" }, 403))
    await expect(chatSessionService.list()).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message:
        "You don't have access. This needs one of these permissions: assistant:use, assistant:ask.",
    })
  })

  it("a refused delete rejects with the same sentence", async () => {
    stubFetch(() => json({ error: "permission_denied: assistant:use, assistant:ask" }, 403))
    await expect(chatSessionService.remove("s1")).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message:
        "You don't have access. This needs one of these permissions: assistant:use, assistant:ask.",
    })
  })

  it("a missing conversation still reads the fixed sentence, not the body's text", async () => {
    stubFetch(() => json({ error: "sesi tidak ditemukan" }, 404))
    await expect(chatSessionService.get("s1")).rejects.toMatchObject({
      code: "not_found",
      status: 404,
      message: "This conversation could not be found. It may have been deleted.",
    })
  })
})
