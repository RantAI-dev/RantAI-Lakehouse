import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { semanticService } from "./semantic"

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

const ENTRY = {
  asset: "serving.mart_x",
  column: "order_id",
  description: "One row per order",
  synonyms: ["order"],
  role: "key",
  status: "draft",
  writtenBy: null,
  model: "m",
  updatedAt: "2026-10-07T10:00:00Z",
}

describe("semanticService.getEntries", () => {
  it("gets the entries of an asset from /api/semantic and returns them as they came", async () => {
    const calls = stubFetch(() => json({ entries: [ENTRY] }))
    const answer = await semanticService.getEntries("serving.mart_x")
    expect(answer).toEqual({ supported: true, entries: [ENTRY] })
    expect(calls).toHaveLength(1)
    expect(calls[0]!.url).toBe("/api/semantic/serving.mart_x")
    expect(calls[0]!.init.method ?? "GET").toBe("GET")
  })

  it("returns the unsupported answer with its reason and does not throw", async () => {
    stubFetch(() => json({ supported: false, reason: "r" }))
    expect(await semanticService.getEntries("serving.mart_x")).toEqual({ supported: false, reason: "r" })
  })
})

describe("semanticService.saveEntry", () => {
  it("puts a column with a role as exactly the column, description, synonyms and role", async () => {
    const calls = stubFetch(() => json({ ok: true }))
    await semanticService.saveEntry("serving.mart_x", {
      column: "order_id",
      description: "One row per order",
      synonyms: ["order"],
      role: "key",
    })
    expect(calls[0]!.url).toBe("/api/semantic/serving.mart_x")
    expect(calls[0]!.init.method).toBe("PUT")
    expect(JSON.parse(String(calls[0]!.init.body))).toEqual({
      column: "order_id",
      description: "One row per order",
      synonyms: ["order"],
      role: "key",
    })
  })

  it("omits the role key when a column's role is null", async () => {
    const calls = stubFetch(() => json({ ok: true }))
    await semanticService.saveEntry("serving.mart_x", {
      column: "order_id",
      description: "One row per order",
      synonyms: [],
      role: null,
    })
    const body = JSON.parse(String(calls[0]!.init.body)) as Record<string, unknown>
    expect(Object.keys(body)).not.toContain("role")
    expect(body.column).toBe("order_id")
  })

  it("omits the column and role keys when saving the table's own entry", async () => {
    const calls = stubFetch(() => json({ ok: true }))
    await semanticService.saveEntry("serving.mart_x", {
      column: null,
      description: "Orders",
      synonyms: ["sales"],
      role: null,
    })
    const body = JSON.parse(String(calls[0]!.init.body)) as Record<string, unknown>
    expect(Object.keys(body)).not.toContain("column")
    expect(Object.keys(body)).not.toContain("role")
    expect(body).toEqual({ description: "Orders", synonyms: ["sales"] })
  })

  it("rejects with invalid_request and the route's own message when a limit is broken", async () => {
    stubFetch(() => json({ error: "`synonyms` must be at most 6" }, 400))
    try {
      await semanticService.saveEntry("serving.mart_x", {
        column: "order_id",
        description: "d",
        synonyms: [],
        role: null,
      })
      throw new Error("expected the call to be refused")
    } catch (err) {
      expect(err).toBeInstanceOf(ServiceError)
      expect((err as ServiceError).code).toBe("invalid_request")
      expect((err as ServiceError).message).toBe("`synonyms` must be at most 6")
    }
  })

  it("rejects with permission_denied and the reason when a 200 answers supported false", async () => {
    stubFetch(() => json({ supported: false, reason: "r" }))
    try {
      await semanticService.saveEntry("serving.mart_x", {
        column: null,
        description: "d",
        synonyms: [],
        role: null,
      })
      throw new Error("expected the call to be refused")
    } catch (err) {
      expect(err).toBeInstanceOf(ServiceError)
      expect((err as ServiceError).code).toBe("permission_denied")
      expect((err as ServiceError).message).toBe("r")
    }
  })
})
