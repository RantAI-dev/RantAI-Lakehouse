import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { notificationsService } from "./notifications"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
})

type Call = { url: string; init: RequestInit }

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

function stubFetch(answer: () => Response): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(input), init: init ?? {} })
    return answer()
  }) as unknown as typeof fetch
  return calls
}

const sent = (call: Call) => JSON.parse(String(call.init.body))

describe("marking mentions read (BI-38 D5)", () => {
  it("marks one dashboard's mentions with its board in the body", async () => {
    const calls = stubFetch(() => json({ marked: 1 }))
    await notificationsService.markMentionsRead("b_1")
    expect(calls[0].url).toBe("/api/dashboard/comments/mentions/read")
    expect(calls[0].init.method).toBe("POST")
    expect(sent(calls[0])).toEqual({ board: "b_1" })
  })

  it("marks every mention with an empty body and no board", async () => {
    const calls = stubFetch(() => json({ marked: 2 }))
    await notificationsService.markMentionsRead()
    expect(sent(calls[0])).toEqual({})
  })

  it("rejects when the server refuses, marking nothing", async () => {
    stubFetch(() => json({ error: "dashboard not found." }, 404))
    const failure = notificationsService.markMentionsRead("b_hidden")
    await expect(failure).rejects.toBeInstanceOf(ServiceError)
    // The sender's sentence is fixed: the two callers (the bell and the
    // comments panel) treat a failure as "still unread", so it stays ours.
    await expect(failure).rejects.toThrow("The mentions could not be marked as read")
  })
})

describe("notification refusals (WEB-12)", () => {
  it("the notification list rejects a refusal as permission_denied, not unavailable", async () => {
    stubFetch(() => json({ error: "permission_denied: dashboard:read" }, 403))
    await expect(notificationsService.list()).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the dashboard:read permission.",
    })
  })

  it("marking mentions read that is refused rejects as permission_denied with the sentence", async () => {
    stubFetch(() => json({ error: "permission_denied: dashboard:write" }, 403))
    await expect(notificationsService.markMentionsRead("b1")).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the dashboard:write permission.",
    })
  })
})
