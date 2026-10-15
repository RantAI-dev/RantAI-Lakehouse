import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { calcFieldService } from "./calc-fields"

const originalFetch = global.fetch
beforeEach(() => localStorage.clear())
afterEach(() => {
  global.fetch = originalFetch
  localStorage.clear()
})

type Call = { url: string; init: RequestInit }
const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

function stubFetch(answer: () => Response): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(input), init: init ?? {} })
    return answer()
  }) as unknown as typeof fetch
  return calls
}

describe("calculated fields client", () => {
  it("lists the fields of a mart and of a SQL source by their own query parameter", async () => {
    const calls = stubFetch(() => json({ fields: [{ id: "f_1", name: "profit" }] }))
    const got = await calcFieldService.list({ mart: "mart_x" })
    expect(got[0].name).toBe("profit")
    await calcFieldService.list({ source: "s_ab12" })
    expect(calls[0].url).toBe("/api/dashboard/calc-fields?mart=mart_x")
    expect(calls[1].url).toBe("/api/dashboard/calc-fields?source=s_ab12")
  })

  it("validate posts the source, the formula and the name, and returns a mistake as data", async () => {
    const calls = stubFetch(() => json({ ok: false, error: { message: "nope", position: 3, length: 1 } }))
    const got = await calcFieldService.validate({ source: "s_1" }, "a +", "profit")
    expect(got).toEqual({ ok: false, error: { message: "nope", position: 3, length: 1 } })
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ source: "s_1", formula: "a +", name: "profit" })
  })

  it("create and update send their bodies and return the saved field", async () => {
    const calls = stubFetch(() => json({ ok: true, field: { id: "f_1", name: "profit" } }))
    await calcFieldService.create({ mart: "mart_x" }, "profit", "[a] - [b]")
    await calcFieldService.update("f_1", "[a] + [b]")
    expect(calls[0].init.method).toBe("POST")
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ mart: "mart_x", name: "profit", formula: "[a] - [b]" })
    expect(calls[1].init.method).toBe("PUT")
    expect(JSON.parse(String(calls[1].init.body))).toEqual({ id: "f_1", formula: "[a] + [b]" })
  })

  it("a refusal carries the server's sentence as invalid_request, a missing permission as permission_denied", async () => {
    stubFetch(() => json({ error: "the field is still used by chart c_1." }, 409))
    const err = await calcFieldService.remove("f_1").catch((e) => e)
    expect(err).toBeInstanceOf(ServiceError)
    expect(err.code).toBe("invalid_request")
    expect(err.message).toContain("still used")
    stubFetch(() => json({ error: "permission_denied: dashboard:write" }, 403))
    const denied = await calcFieldService.create({ mart: "m" }, "x", "1").catch((e) => e)
    expect(denied.code).toBe("permission_denied")
  })

  it("functions returns the catalog", async () => {
    stubFetch(() => json({ functions: [{ name: "Sum" }] }))
    expect((await calcFieldService.functions())[0].name).toBe("Sum")
  })
})
