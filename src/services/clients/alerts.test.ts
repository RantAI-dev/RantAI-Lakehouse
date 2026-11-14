import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { clickhouseAlertRuleService } from "./alerts"

// `BI-40` T4: a person tests one rule through `/api/alerts/test`. The
// scheduler's `POST /api/alerts/run` is a separate door this client must never
// call, and this route's refusals are a permission or a missing rule — never a
// lost session.

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
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

async function rejection(run: Promise<unknown>): Promise<ServiceError> {
  try {
    await run
  } catch (err) {
    expect(err).toBeInstanceOf(ServiceError)
    return err as ServiceError
  }
  throw new Error("expected the call to be refused")
}

const RULE = "al_1234"

describe("clickhouseAlertRuleService.testRule", () => {
  it("posts to /api/alerts/test with the rule id, and never to the scheduler's /api/alerts/run", async () => {
    const calls = stubFetch(() => json({ ran: 1, results: [] }))

    const answer = await clickhouseAlertRuleService.testRule(RULE)

    expect(answer).toEqual({ ran: 1, results: [] })
    expect(calls).toHaveLength(1)
    expect(calls[0]!.url).toBe(`/api/alerts/test?id=${RULE}`)
    expect(calls[0]!.init.method).toBe("POST")
    expect(calls.some((call) => call.url.startsWith("/api/alerts/run"))).toBe(false)
  })

  it("says the alert permission in words on a 403", async () => {
    stubFetch(() => json({ error: "permission_denied: alert:write" }, 403))

    const denied = await rejection(clickhouseAlertRuleService.testRule(RULE))

    expect(denied.code).toBe("permission_denied")
    expect(denied.message).toBe("You can test only the alerts you manage.")
  })

  it("keeps the server's sentence when the rule is gone or not the caller's", async () => {
    stubFetch(() => json({ error: "alert rule not found." }, 404))

    const missing = await rejection(clickhouseAlertRuleService.testRule(RULE))

    expect(missing.code).toBe("not_found")
    expect(missing.message).toBe("alert rule not found.")
  })
})

describe("clickhouseAlertRuleService refusals (WEB-12)", () => {
  it("listRules rejects with permission_denied, the status and the sentence", async () => {
    stubFetch(() => json({ error: "permission_denied: alert:read" }, 403))

    await expect(clickhouseAlertRuleService.listRules()).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the alert:read permission.",
    })
  })
})
