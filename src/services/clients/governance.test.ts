import { afterEach, describe, expect, it, mock } from "bun:test"
import type { AuditEvent, AuditActorsPage } from "../contracts/governance"
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

describe("governanceService: row filter grammar (WEB-8 D8; S2 review round 1, S2)", () => {
  it("reads the functions, operators and forms the server's validator serves", async () => {
    const grammar = {
      functions: ["lower", "upper"],
      operators: ["=", "AND"],
      forms: ["LIKE", "BETWEEN"],
    }
    const calls = stubFetch(() => json(grammar))
    expect(await service.getRowFilterGrammar()).toEqual(grammar)
    expect(calls[0].url).toBe("/api/governance/row-filter-grammar")
    expect(calls[0].init.method ?? "GET").toBe("GET")
  })

  it("maps a refused read onto the error the API's sentence names", async () => {
    stubFetch(() => json({ error: "permission_denied: policy:read" }, 403))
    await expect(service.getRowFilterGrammar()).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the policy:read permission.",
    })
  })
})

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

describe("governanceService refusals (TC-1724, WEB-12)", () => {
  const helpers: [string, () => Promise<unknown>][] = [
    ["listQuality", () => service.listQuality()],
    ["createPolicy", () => service.createPolicy({} as never)],
    ["updateQualityRule", () => service.updateQualityRule("r1", {} as never)],
    ["deletePolicy", () => service.deletePolicy("p1")],
    ["deleteQualityRule", () => service.deleteQualityRule("r1")],
    ["deleteClassificationRule", () => service.deleteClassificationRule("c1")],
  ]

  it("every governance helper rejects a refusal with permission_denied, the status and the sentence", async () => {
    for (const [, call] of helpers) {
      stubFetch(() => json({ error: "permission_denied: governance:write" }, 403))
      await expect(call()).rejects.toMatchObject({
        code: "permission_denied",
        status: 403,
        message: "You don't have access. This needs the governance:write permission.",
      })
    }
  })
})

// GOV-1 part B, decisions 5 and 9: the audit page searches on the server, opens
// one event by id and exports the same search as a CSV file.
describe("governanceService: audit search, one event and export (GOV-1)", () => {
  const sent = (url: string) => {
    const [path, query = ""] = url.split("?")
    return { path, params: Object.fromEntries(new URLSearchParams(query)) }
  }
  const row: AuditEvent = {
    id: "audit-1",
    at: "2026-11-06T08:00:00.000Z",
    actor: "Platform Admin",
    actorId: "usr_admin",
    actorKind: "user",
    action: "dashboard.share",
    resource: "dashboard:d1",
    outcome: "success",
    outcomeDetail: "executed",
    policyDecision: "allow",
    obligations: [],
    reason: null,
  }

  it("sends every filter the person set, and no key for an empty one", async () => {
    const calls = stubFetch(() => json({ events: [row], nextCursor: "c2" }))
    const page = await service.searchAudit({
      actor: "u-1",
      action: "dashboard.",
      resourceKind: "dashboard",
      resourceId: "d 1",
      outcome: "refused",
      from: "2026-11-01T00:00:00Z",
      to: "2026-11-02T00:00:00Z",
      limit: 50,
      cursor: "abc",
    })
    expect(page).toEqual({ events: [row], nextCursor: "c2" })
    expect(sent(calls[0].url)).toEqual({
      path: "/api/governance/audit/events",
      params: {
        actor: "u-1",
        action: "dashboard.",
        resourceKind: "dashboard",
        resourceId: "d 1",
        outcome: "refused",
        from: "2026-11-01T00:00:00Z",
        to: "2026-11-02T00:00:00Z",
        limit: "50",
        cursor: "abc",
      },
    })

    const bare = stubFetch(() => json({ events: [], nextCursor: null }))
    await service.searchAudit({ actor: "", outcome: undefined })
    expect(bare[0].url).toBe("/api/governance/audit/events")
  })

  it("reads the trail's principals from the actors route, with no params, and refuses like its neighbours", async () => {
    const actors: AuditActorsPage = {
      actors: [
        { actorId: "usr_admin", actor: "Platform Admin", actorKind: "user" },
        { actorId: "agt_qa", actor: "QA Runner", actorKind: "agent" },
      ],
    }
    const calls = stubFetch(() => json(actors))
    expect(await service.listAuditActors()).toEqual(actors)
    expect(sent(calls[0].url)).toEqual({ path: "/api/governance/audit/actors", params: {} })

    stubFetch(() => json({ error: "permission_denied: audit:read" }, 403))
    await expect(service.listAuditActors()).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the audit:read permission.",
    })
    stubFetch(() => json({ error: "The audit trail could not be read." }, 500))
    await expect(service.listAuditActors()).rejects.toMatchObject({ code: "unavailable" })
  })

  it("maps a refusal and a bad filter onto the sentence the API gave", async () => {
    stubFetch(() => json({ error: "permission_denied: audit:read" }, 403))
    await expect(service.searchAudit({})).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the audit:read permission.",
    })
    stubFetch(() => json({ error: "from must be before to." }, 400))
    await expect(service.searchAudit({})).rejects.toMatchObject({
      code: "invalid_request",
      message: "from must be before to.",
    })
  })

  it("reads one event by id, and answers null when the API says it is not there", async () => {
    const calls = stubFetch(() => json({ event: row }))
    expect(await service.getAuditEvent("audit/1 x")).toEqual(row)
    expect(calls[0].url).toBe("/api/governance/audit/events/audit%2F1%20x")

    stubFetch(() => json({ error: "No audit event has that id." }, 404))
    expect(await service.getAuditEvent("gone")).toBeNull()

    stubFetch(() => json({ error: "permission_denied: audit:read" }, 403))
    await expect(service.getAuditEvent("a")).rejects.toMatchObject({ code: "permission_denied" })
  })

  it("exports with the same filters, no cursor and no limit, and reads the file name", async () => {
    const calls = stubFetch(
      () =>
        new Response("at,actor\n", {
          status: 200,
          headers: {
            "content-type": "text/csv; charset=utf-8",
            "content-disposition": 'attachment; filename="audit-events-2026-11-15.csv"',
          },
        }),
    )
    const file = await service.exportAuditCsv({
      actor: "u-1",
      outcome: "failed",
      from: "2026-11-01T00:00:00Z",
      limit: 50,
      cursor: "abc",
    })
    expect(sent(calls[0].url)).toEqual({
      path: "/api/governance/audit/export.csv",
      params: { actor: "u-1", outcome: "failed", from: "2026-11-01T00:00:00Z" },
    })
    expect(file.fileName).toBe("audit-events-2026-11-15.csv")
    expect(await file.blob.text()).toBe("at,actor\n")
    expect(file.truncated).toBe(false)
    expect(file.total).toBeNull()
  })

  it("says when the server cut the file, with how many events matched", async () => {
    stubFetch(
      () =>
        new Response("at\n", {
          status: 200,
          headers: { "x-audit-export-truncated": "true", "x-audit-export-total": "250000" },
        }),
    )
    const file = await service.exportAuditCsv({})
    expect(file.truncated).toBe(true)
    expect(file.total).toBe(250000)
    expect(file.fileName).toBe("audit-events.csv")
  })

  it("rejects a refused export with the API's sentence", async () => {
    stubFetch(() => json({ error: "permission_denied: audit:read" }, 403))
    await expect(service.exportAuditCsv({})).rejects.toMatchObject({ code: "permission_denied", status: 403 })
  })
})
