// The Gold client is the only place the console learns who read a
// published table (DATA-5 D7/D9). These tests pin the URL the count
// comes from and the two honest shapes the API answers, so a contract
// change cannot reach the page unannounced.

import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { goldService } from "./gold"

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

function stubFetch(answer: () => Response): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(input), init: init ?? {} })
    return answer()
  }) as unknown as typeof fetch
  return calls
}

describe("goldService.getConsumers", () => {
  it("asks the mart's consumers route and returns the counted tools", async () => {
    const calls = stubFetch(() =>
      json({
        mart: "sales",
        supported: true,
        windowDays: 7,
        consumers: [
          {
            identityId: "11111111-1111-1111-1111-111111111111",
            name: "Finance DuckDB",
            reads: 4,
            lastReadAt: "2026-10-31T08:00:00Z",
          },
        ],
      }),
    )

    const result = await goldService.getConsumers("sales")

    expect(calls).toHaveLength(1)
    expect(calls[0].url).toBe("/api/gold/export/sales/consumers")
    expect(result.supported).toBe(true)
    expect(result.windowDays).toBe(7)
    expect(result.consumers?.map((c) => c.name)).toEqual(["Finance DuckDB"])
    expect(result.consumers?.[0].reads).toBe(4)
  })

  it("keeps the refusal shape, with its reason, when outside reading is not set up", async () => {
    stubFetch(() =>
      json({
        mart: "sales",
        supported: false,
        consumers: null,
        reason: "Outside reading is not set up: OUTSIDE_S3_ENDPOINT is not set.",
      }),
    )

    const result = await goldService.getConsumers("sales")

    expect(result.supported).toBe(false)
    expect(result.consumers).toBeNull()
    expect(result.reason).toBe(
      "Outside reading is not set up: OUTSIDE_S3_ENDPOINT is not set.",
    )
  })

  it("turns a non-OK answer into a ServiceError instead of a fake count", async () => {
    stubFetch(() => json({ error: "database error" }, 500))

    const err = await goldService.getConsumers("sales").catch((e) => e)

    expect(err).toBeInstanceOf(ServiceError)
    expect(err.code).toBe("unavailable")
  })
})
