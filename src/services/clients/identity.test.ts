// GOV-7 S2 (D7/D8): the two reads/writes the console's role page and its
// roles list use for "tables this role can read". The body and the answer
// are the API's own (`routes::identity::get_role_table_access` /
// `put_role_table_access`), and a refusal keeps the server's sentence.
import { afterEach, describe, expect, it, mock } from "bun:test"
import { postgresIdentityService as service } from "./identity"

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

describe("identityService: a role's table access (GOV-7 D7)", () => {
  it("reads a role's scope and grants", async () => {
    const calls = stubFetch(() =>
      json({ scope: "listed", grants: [{ schema: "finance", table: "*" }] })
    )
    const access = await service.getRoleTableAccess("r1")
    expect(access).toEqual({ scope: "listed", grants: [{ schema: "finance", table: "*" }] })
    expect(calls[0].url).toBe("/api/identity/roles/r1/table-access")
    expect(calls[0].init.method).toBeUndefined()
  })

  it("writes the complete scope and grants with a PUT", async () => {
    const calls = stubFetch(() =>
      json({ scope: "listed", grants: [{ schema: "silver", table: "orders" }] })
    )
    await service.setRoleTableAccess("r1", {
      scope: "listed",
      grants: [{ schema: "silver", table: "orders" }],
    })
    expect(calls[0].url).toBe("/api/identity/roles/r1/table-access")
    expect(calls[0].init.method).toBe("PUT")
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      scope: "listed",
      grants: [{ schema: "silver", table: "orders" }],
    })
  })

  it("keeps the API's sentence when a *:* role is refused", async () => {
    stubFetch(() => json({ error: "A role with full access cannot be narrowed" }, 400))
    await expect(
      service.setRoleTableAccess("r1", { scope: "listed", grants: [] })
    ).rejects.toMatchObject({
      code: "invalid_request",
      message: "A role with full access cannot be narrowed",
    })
  })

  it("maps a missing role onto not_found", async () => {
    stubFetch(() => json({ error: "role not found" }, 404))
    await expect(service.getRoleTableAccess("nope")).rejects.toMatchObject({
      code: "not_found",
    })
  })
})
