import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { chatTermService } from "./chat-terms"

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

const STORED = {
  term: "active customer",
  meaning: "Ordered in the last 30 days",
  question: "Show active customers",
  updatedAt: "2026-10-07T10:00:00Z",
}

describe("chatTermService.saveTerm", () => {
  it("puts the term, the meaning and the question as JSON to /api/ai/terms and returns the stored term", async () => {
    const calls = stubFetch(() => json(STORED))
    const stored = await chatTermService.saveTerm({
      term: "active customer",
      meaning: "Ordered in the last 30 days",
      question: "Show active customers",
    })
    expect(stored).toEqual(STORED)
    expect(calls[0]!.url).toBe("/api/ai/terms")
    expect(calls[0]!.init.method).toBe("PUT")
    expect(JSON.parse(String(calls[0]!.init.body))).toEqual({
      term: "active customer",
      meaning: "Ordered in the last 30 days",
      question: "Show active customers",
    })
  })

  it("rejects with the server's message when it refuses the term", async () => {
    stubFetch(() => json({ error: "term is too long" }, 400))
    try {
      await chatTermService.saveTerm({ term: "x", meaning: "y" })
      throw new Error("expected the call to be refused")
    } catch (err) {
      expect(err).toBeInstanceOf(ServiceError)
      expect((err as ServiceError).message).toBe("term is too long")
      expect((err as ServiceError).code).toBe("invalid_request")
    }
  })
})
