import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { clickhouseAssetService } from "./assets"

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

describe("clickhouseAssetService.setCertification", () => {
  it("puts the status, note and replacement as JSON to the asset's certification route", async () => {
    const calls = stubFetch(() => json({ ok: true }))
    await clickhouseAssetService.setCertification!("silver.orders", {
      status: "deprecated",
      note: "Superseded",
      replacementAssetId: "silver.orders_v2",
    })
    expect(calls).toHaveLength(1)
    expect(calls[0].url).toBe("/api/catalog/silver.orders/certification")
    expect(calls[0].init.method).toBe("PUT")
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      status: "deprecated",
      note: "Superseded",
      replacementAssetId: "silver.orders_v2",
    })
  })

  it("sends status null to clear the mark", async () => {
    const calls = stubFetch(() => json({ ok: true }))
    await clickhouseAssetService.setCertification!("silver.orders", { status: null })
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ status: null })
  })

  it("throws the API's sentence as an invalid_request on a 400", async () => {
    stubFetch(() => json({ error: "the replacement is itself deprecated" }, 400))
    const err = await clickhouseAssetService
      .setCertification!("silver.orders", { status: "deprecated", replacementAssetId: "x" })
      .catch((e) => e)
    expect(err).toBeInstanceOf(ServiceError)
    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe("the replacement is itself deprecated")
  })

  it("throws permission_denied on a 403", async () => {
    stubFetch(() => json({ error: "permission_denied: governance:write" }, 403))
    const err = await clickhouseAssetService.setCertification!("silver.orders", { status: "certified" }).catch((e) => e)
    expect(err.code).toBe("permission_denied")
  })
})
