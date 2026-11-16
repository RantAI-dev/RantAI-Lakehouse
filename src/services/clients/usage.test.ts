import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { usageService } from "./usage"

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

const TOTALS = {
  key: "",
  label: "",
  queries: 3,
  completed: 2,
  failed: 1,
  blocked: 0,
  cancelled: 0,
  scannedBytes: 4096,
  engineMs: 1500,
  estimate: null,
}

const REPORT = {
  from: "2026-10-01",
  to: "2026-10-30",
  group: "day",
  scope: "tenant",
  currency: null,
  rows: [{ ...TOTALS, key: "2026-10-01", label: "2026-10-01" }],
  totals: TOTALS,
}

const QUERY = { from: "2026-10-01", to: "2026-10-30", group: "day" }

const RATE = {
  id: "r1",
  kind: "scan_tb",
  currency: "USD",
  amount: "5.000000",
  effectiveFrom: "2026-10-01",
  createdAt: "2026-10-01T08:00:00Z",
}

describe("usageService", () => {
  it("reads the sums from /api/usage/queries with the period and group, as they came", async () => {
    const calls = stubFetch(() => json(REPORT))
    expect(await usageService.getUsage(QUERY)).toEqual(REPORT)
    const url = new URL(calls[0].url, "http://console.test")
    expect(url.pathname).toBe("/api/usage/queries")
    expect(Object.fromEntries(url.searchParams)).toEqual({
      from: "2026-10-01",
      to: "2026-10-30",
      group: "day",
    })
  })

  it("sends tenant=all only when the query asks for it", async () => {
    const calls = stubFetch(() => json(REPORT))
    await usageService.getUsage({ ...QUERY, group: "tenant", tenant: "all" })
    const url = new URL(calls[0].url, "http://console.test")
    expect(url.searchParams.get("tenant")).toBe("all")
    expect(url.searchParams.get("group")).toBe("tenant")
  })

  it("maps a refusal to a permission error with the API's sentence", async () => {
    stubFetch(() =>
      json({ error: "permission_denied: Only an administrator of the whole deployment may ask for all tenants." }, 403)
    )
    const err = await rejection(usageService.getUsage({ ...QUERY, tenant: "all" }))
    expect(err.code).toBe("permission_denied")
    expect(err.status).toBe(403)
  })

  it("keeps the fixed sentence of a bad request", async () => {
    stubFetch(() => json({ error: "A period is at most 366 days." }, 400))
    const err = await rejection(usageService.getUsage(QUERY))
    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe("A period is at most 366 days.")
  })

  it("downloads the CSV from /api/usage/queries/export with the same parameters and the server's file name", async () => {
    const calls = stubFetch(
      () =>
        new Response("day,queries\n2026-10-01,3\n", {
          status: 200,
          headers: {
            "content-type": "text/csv; charset=utf-8",
            "content-disposition": 'attachment; filename="usage-2026-10-01-2026-10-30.csv"',
          },
        })
    )
    const file = await usageService.exportUsageCsv(QUERY)
    const url = new URL(calls[0].url, "http://console.test")
    expect(url.pathname).toBe("/api/usage/queries/export")
    expect(url.searchParams.get("group")).toBe("day")
    expect(file.fileName).toBe("usage-2026-10-01-2026-10-30.csv")
    expect(file.blob).toBeInstanceOf(Blob)
    expect(await file.blob.text()).toBe("day,queries\n2026-10-01,3\n")
  })

  it("falls back to a usage file name when the server names none", async () => {
    stubFetch(() => new Response("day\n", { status: 200 }))
    const file = await usageService.exportUsageCsv(QUERY)
    expect(file.fileName).toBe("usage-2026-10-01-2026-10-30.csv")
  })

  it("refuses a download with the API's sentence", async () => {
    stubFetch(() => json({ error: "from must not be after to." }, 400))
    const err = await rejection(usageService.exportUsageCsv(QUERY))
    expect(err.message).toBe("from must not be after to.")
  })

  it("lists the rates from GET /api/usage/rates", async () => {
    const calls = stubFetch(() => json({ currency: "USD", rates: [RATE] }))
    expect(await usageService.listRates()).toEqual({ currency: "USD", rates: [RATE] })
    expect(calls[0].url).toBe("/api/usage/rates")
    expect(calls[0].init.method ?? "GET").toBe("GET")
  })

  it("adds a rate with a POST carrying the camelCase body", async () => {
    const calls = stubFetch(() => json(RATE, 201))
    const stored = await usageService.addRate({
      kind: "scan_tb",
      currency: "USD",
      amount: "5",
      effectiveFrom: "2026-10-01",
    })
    expect(stored).toEqual(RATE)
    expect(calls[0].url).toBe("/api/usage/rates")
    expect(calls[0].init.method).toBe("POST")
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      kind: "scan_tb",
      currency: "USD",
      amount: "5",
      effectiveFrom: "2026-10-01",
    })
  })

  it("keeps the sentence of a second currency", async () => {
    stubFetch(() =>
      json({ error: "Rates use one currency. The rates already entered use another one, so this rate was not added." }, 400)
    )
    const err = await rejection(
      usageService.addRate({ kind: "scan_tb", currency: "EUR", amount: "5", effectiveFrom: "2026-10-01" })
    )
    expect(err.code).toBe("invalid_request")
    expect(err.message).toContain("one currency")
  })

  it("maps a refused rate write to a permission error", async () => {
    stubFetch(() => json({ error: "permission_denied: settings:write" }, 403))
    const err = await rejection(
      usageService.addRate({ kind: "scan_tb", currency: "USD", amount: "5", effectiveFrom: "2026-10-01" })
    )
    expect(err.code).toBe("permission_denied")
  })
})
