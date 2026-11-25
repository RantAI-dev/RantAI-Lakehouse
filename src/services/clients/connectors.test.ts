import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { postgresConnectorService } from "./connectors"

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

async function rejection(run: Promise<unknown>): Promise<ServiceError> {
  try {
    await run
  } catch (err) {
    if (err instanceof ServiceError) return err
    throw new Error(`expected ServiceError, got: ${String(err)}`)
  }
  throw new Error("expected rejection, call resolved")
}

// QA LH-DATA-9-20 (WEB-11): Sources said "Request could not be processed" and
// did not name the missing permission.
describe("postgresConnectorService refusals", () => {
  it("listConnectors throws permission_denied with the sentence that names connector:manage on a 403", async () => {
    stubFetch(() => json({ error: "permission_denied: connector:manage" }, 403))

    const err = await rejection(postgresConnectorService.listConnectors())
    expect(err.code).toBe("permission_denied")
    expect(err.status).toBe(403)
    expect(err.message).toBe("You don't have access. This needs the connector:manage permission.")
  })

  it("a refused test, a refused tenant move, a refused log download and a refused delete are all permission_denied with the status", async () => {
    const calls = [
      () => postgresConnectorService.testConnection("conn-a"),
      () => postgresConnectorService.assignTenant("conn-a", "t1"),
      () => postgresConnectorService.downloadConnectorRunLog("conn-a", "run-1"),
      () => postgresConnectorService.deleteConnector("conn-a"),
    ]
    for (const call of calls) {
      stubFetch(() => json({ error: "permission_denied: connector:manage" }, 403))
      const err = await rejection(call())
      expect(err.code).toBe("permission_denied")
      expect(err.status).toBe(403)
      expect(err.message).not.toContain("permission_denied")
    }
  })

  it("still calls a 404 not found, a 409 a bad request and a 503 an outage, with the API's sentence", async () => {
    stubFetch(() => json({ error: "A sentence." }, 404))
    const missing = await rejection(postgresConnectorService.getConnector("conn-a"))
    expect(missing.code).toBe("not_found")
    expect(missing.message).toBe("A sentence.")

    stubFetch(() => json({ error: "A sentence." }, 409))
    const conflict = await rejection(postgresConnectorService.getConnector("conn-a"))
    expect(conflict.code).toBe("invalid_request")
    expect(conflict.message).toBe("A sentence.")

    stubFetch(() => json({ error: "A sentence." }, 503))
    const down = await rejection(postgresConnectorService.getConnector("conn-a"))
    expect(down.code).toBe("unavailable")
    expect(down.message).toBe("A sentence.")
  })
})
