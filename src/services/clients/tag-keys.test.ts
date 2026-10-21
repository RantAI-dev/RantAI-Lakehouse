import { afterEach, describe, expect, it, mock } from "bun:test"
import { tagKeysService } from "./tag-keys"

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

const KEY = {
  key: "data_domain",
  allowedValues: ["finance", "ops"],
  createdBy: "Rina",
  createdAt: "2026-10-14T10:00:00Z",
  updatedAt: "2026-10-14T10:00:00Z",
}

describe("tagKeysService", () => {
  it("lists the keys from /api/governance/tag-keys as they came", async () => {
    const calls = stubFetch(() => json([KEY]))
    expect(await tagKeysService.list()).toEqual([KEY])
    expect(calls[0].url).toBe("/api/governance/tag-keys")
  })

  it("defines a key with a POST carrying the key and its values", async () => {
    const calls = stubFetch(() => json(KEY))
    await tagKeysService.create("data_domain", ["finance", "ops"])
    expect(calls[0].init.method).toBe("POST")
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      key: "data_domain",
      allowedValues: ["finance", "ops"],
    })
  })

  it("changes the values with a PUT to the encoded key and sends no new key", async () => {
    const calls = stubFetch(() => json(KEY))
    await tagKeysService.update("data domain", ["finance"])
    expect(calls[0].url).toBe("/api/governance/tag-keys/data%20domain")
    expect(calls[0].init.method).toBe("PUT")
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ allowedValues: ["finance"] })
  })

  it("removes a key with a DELETE", async () => {
    const calls = stubFetch(() => json({ ok: true, key: "data_domain" }))
    await tagKeysService.remove("data_domain")
    expect(calls[0].init.method).toBe("DELETE")
    expect(calls[0].url).toBe("/api/governance/tag-keys/data_domain")
  })

  it("rejects with the API's own sentence and the code its status implies", async () => {
    stubFetch(() => json({ error: "\"data_domain\" is still given to 2 tables; remove it from those tables first" }, 400))
    await expect(tagKeysService.remove("data_domain")).rejects.toMatchObject({
      code: "invalid_request",
      message: "\"data_domain\" is still given to 2 tables; remove it from those tables first",
    })

    stubFetch(() => json({ error: "the tag key \"k\" already exists" }, 409))
    await expect(tagKeysService.create("k", ["a"])).rejects.toMatchObject({ code: "invalid_request" })

    stubFetch(() => json({ error: "permission_denied: governance:write" }, 403))
    await expect(tagKeysService.create("k", ["a"])).rejects.toMatchObject({ code: "permission_denied" })

    stubFetch(() => json({ error: "the tag key \"k\" is not defined" }, 404))
    await expect(tagKeysService.update("k", ["a"])).rejects.toMatchObject({ code: "not_found" })
  })
})
