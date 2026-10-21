import { afterEach, describe, expect, it, mock } from "bun:test"
import { clickhouseGovernanceService as service } from "./governance"

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

describe("governanceService: quality incidents, schedules and history (DATA-14)", () => {
  it("lists open incidents by default and narrows by status and table", async () => {
    const calls = stubFetch(() => json({ incidents: [] }))
    await service.listQualityIncidents()
    await service.listQualityIncidents({ status: "all", asset: "silver.orders" })
    expect(calls.map((c) => c.url)).toEqual([
      "/api/governance/quality/incidents",
      "/api/governance/quality/incidents?status=all&asset=silver.orders",
    ])
  })

  it("resolves an incident with a POST carrying the reason", async () => {
    const calls = stubFetch(() => json({ id: "i1", status: "resolved" }))
    await service.resolveQualityIncident("i1", "Source fixed")
    expect(calls[0].url).toBe("/api/governance/quality/incidents/i1/resolve")
    expect(calls[0].init.method).toBe("POST")
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ resolution: "Source fixed" })
  })

  it("maps a refused resolve onto the error the API's sentence names", async () => {
    stubFetch(() => json({ error: "That incident is already resolved." }, 409))
    await expect(service.resolveQualityIncident("i1", "x")).rejects.toMatchObject({
      code: "invalid_request",
      message: "That incident is already resolved.",
    })
    stubFetch(() => json({ error: "permission_denied: governance:write" }, 403))
    await expect(service.resolveQualityIncident("i1", "x")).rejects.toMatchObject({
      code: "permission_denied",
    })
  })

  it("reads and sets a table's schedule", async () => {
    const calls = stubFetch(() => json({ schedules: [], defaultIntervalMinutes: 60 }))
    expect((await service.listQualitySchedules()).defaultIntervalMinutes).toBe(60)
    stubFetch(() => json({ asset: "silver.orders", mode: "interval", intervalMinutes: 180 }))
    const set = stubFetch(() => json({ asset: "silver.orders", mode: "interval", intervalMinutes: 180 }))
    await service.setQualitySchedule("silver.orders", { mode: "interval", intervalMinutes: 180 })
    expect(calls[0].url).toBe("/api/governance/quality/schedules")
    expect(set[0].url).toBe("/api/governance/quality/schedules/silver.orders")
    expect(set[0].init.method).toBe("PUT")
    expect(JSON.parse(String(set[0].init.body))).toEqual({ mode: "interval", intervalMinutes: 180 })
  })

  it("reads a table's learning state and a rule's runs", async () => {
    const learning = stubFetch(() => json({ asset: "silver.orders", learned: false }))
    await service.getQualityLearning("silver.orders")
    expect(learning[0].url).toBe("/api/governance/quality/learning/silver.orders")

    const run = { status: "passed" as const, value: "1 rows", runBy: "schedule", at: "2026-10-16T09:00:00Z", threshold: null }
    const runs = stubFetch(() => json({ runs: [run] }))
    expect(await service.listQualityRuns("r 1")).toEqual([run])
    expect(runs[0].url).toBe("/api/governance/quality/r%201/runs")
  })
})
