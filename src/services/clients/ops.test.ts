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

describe("ops service list", () => {
  // GOV-1 Decision 4: the same answer carries the count of audit rows the API
  // could not write since it started, so the page needs no second request.
  it("returns the services and the failed audit write count from one request", async () => {
    const service = { id: "clickhouse", name: "ClickHouse", health: "healthy" }
    const calls = stubFetch(() => json({ services: [service], auditWriteFailures: 3 }))

    const report = await clickhouseOpsService.listServices()

    expect(report.auditWriteFailures).toBe(3)
    expect(report.services.map((row) => row.id)).toEqual([service.id])
    expect(calls.map((c) => c.url)).toEqual(["/api/ops/services"])
  })
})
