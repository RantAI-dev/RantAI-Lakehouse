import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { readableErrorMessage } from "@/lib/notify-message"
import { ServiceError } from "../errors"
import { clickhouseDashboardService } from "./dashboards"

const originalFetch = global.fetch

beforeEach(() => {
  localStorage.clear()
})

afterEach(() => {
  global.fetch = originalFetch
  localStorage.clear()
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

const sent = (call: Call) => JSON.parse(String(call.init.body))

describe("embed state (SEC-12)", () => {
  it("getEmbedInfo reads the board's state from embed-info", async () => {
    const calls = stubFetch(() =>
      json({
        enabled: true,
        supported: true,
        sampleToken: "a.b.c",
        maxLifetimeSeconds: 3600,
        revokedBefore: 1_700_000_000,
        allowedOrigins: ["https://app.example.com"],
      }),
    )
    const info = await clickhouseDashboardService.getEmbedInfo("b 1")
    expect(calls[0].url).toBe("/api/dashboard/embed-info?board=b%201")
    expect(info).toEqual({
      enabled: true,
      supported: true,
      reason: undefined,
      sampleToken: "a.b.c",
      maxLifetimeSeconds: 3600,
      revokedBefore: 1_700_000_000,
      allowedOrigins: ["https://app.example.com"],
    })
  })

  it("getEmbedInfo reports an unconfigured server with its reason and no sample token", async () => {
    stubFetch(() =>
      json({ enabled: false, supported: false, reason: "embedding is not configured", maxLifetimeSeconds: 86400, revokedBefore: null, allowedOrigins: [] }),
    )
    const info = await clickhouseDashboardService.getEmbedInfo("b_1")
    expect(info.supported).toBe(false)
    expect(info.reason).toBe("embedding is not configured")
    expect(info.sampleToken).toBeUndefined()
  })

  it("setEmbedOrigins puts the list on the board and returns what the server stored", async () => {
    const calls = stubFetch(() => json({ ok: true, embedOrigins: ["https://app.example.com"] }))
    const stored = await clickhouseDashboardService.setEmbedOrigins("b_1", ["https://App.example.com"])
    expect(calls[0].url).toBe("/api/dashboard/boards")
    expect(calls[0].init.method).toBe("PUT")
    expect(sent(calls[0])).toEqual({ id: "b_1", embedOrigins: ["https://App.example.com"] })
    expect(stored).toEqual(["https://app.example.com"])
  })

  it("setEmbedOrigins carries the server's refusal as the error message", async () => {
    stubFetch(() => json({ error: '"*" is not an allowed site: wildcards are not accepted; list each site' }, 400))
    const err = await clickhouseDashboardService.setEmbedOrigins("b_1", ["*"]).catch((e: unknown) => e)
    expect(err).toBeInstanceOf(ServiceError)
    expect((err as ServiceError).code).toBe("invalid_request")
    expect((err as ServiceError).message).toContain("wildcards are not accepted")
  })

  it("revokeAllEmbedTokens asks for the withdrawal and returns the stored instant", async () => {
    const calls = stubFetch(() => json({ ok: true, embedRevokedBefore: 1_700_000_123 }))
    const at = await clickhouseDashboardService.revokeAllEmbedTokens("b_1")
    expect(sent(calls[0])).toEqual({ id: "b_1", embedRevokeAll: true })
    expect(at).toBe(1_700_000_123)
  })

  it("revokeAllEmbedTokens without the permission is permission_denied", async () => {
    stubFetch(() => json({ error: "permission_denied: dashboard:write" }, 403))
    const err = await clickhouseDashboardService.revokeAllEmbedTokens("b_1").catch((e: unknown) => e)
    expect((err as ServiceError).code).toBe("permission_denied")
  })

  it("revokeEmbedToken sends the whole token, never a bare id, and returns the withdrawn jti", async () => {
    const calls = stubFetch(() => json({ ok: true, withdrawn: "j-1" }))
    const jti = await clickhouseDashboardService.revokeEmbedToken("b_1", "a.b.c")
    expect(calls[0].url).toBe("/api/dashboard/embed-revoke")
    expect(calls[0].init.method).toBe("POST")
    expect(sent(calls[0])).toEqual({ board: "b_1", token: "a.b.c" })
    expect(jti).toBe("j-1")
  })

  it("revokeEmbedToken says so when the token has no id", async () => {
    stubFetch(() => json({ error: "that token has no id (jti), so it cannot be withdrawn on its own; withdraw all the dashboard's embed tokens instead." }, 400))
    const err = await clickhouseDashboardService.revokeEmbedToken("b_1", "a.b.c").catch((e: unknown) => e)
    expect((err as ServiceError).message).toContain("no id (jti)")
  })
})

describe("a refused dashboard write is an error, not a success (BI-18 part C review fix (SHOULD-FIX) R19)", () => {
  const calls: [string, () => Promise<unknown>][] = [
    ["create", () => clickhouseDashboardService.createBoard({ name: "New dashboard" })],
    ["duplicate", () => clickhouseDashboardService.duplicateBoard("b1")],
    ["rename", () => clickhouseDashboardService.updateBoard({ id: "b1", name: "Renamed" })],
    ["delete a dashboard", () => clickhouseDashboardService.deleteBoard("b1")],
    ["delete a chart", () => clickhouseDashboardService.deleteChart("c1")],
    ["save a layout", () => clickhouseDashboardService.saveBoardLayout("b1", {})],
  ]
  for (const [name, call] of calls) {
    it(`${name} throws the server's sentence on a 403, without the code`, async () => {
      stubFetch(() => json({ error: "permission_denied: dashboard:write" }, 403))
      const err = await call().catch((e: unknown) => e)
      expect(err).toBeInstanceOf(ServiceError)
      expect((err as ServiceError).code).toBe("permission_denied")
      // The page shows the readable sentence: the code is for the log.
      expect(readableErrorMessage((err as ServiceError).message)).toBe(
        "This account does not have permission. Ask a workspace admin.",
      )
    })
  }

  it("says the status when the refusal has no sentence", async () => {
    stubFetch(() => json({}, 500))
    const err = await clickhouseDashboardService.deleteBoard("b1").catch((e: unknown) => e)
    expect((err as ServiceError).message).toBe("The change was refused (500).")
  })

  it("returns the answer of a write that went through", async () => {
    stubFetch(() => json({ board: { id: "b9" } }))
    const copy = await clickhouseDashboardService.duplicateBoard("b1")
    expect(copy.id).toBe("b9")
    stubFetch(() => json({ ok: true }))
    await clickhouseDashboardService.deleteChart("c1")
  })
})

describe("history, trash, personal space, favourites and search (BI-18 part C, part 2)", () => {
  it("lists the versions of a board", async () => {
    const calls = stubFetch(() => json({ versions: [{ version: 2, author: "Ann", at: "t", summary: "Renamed", charts: 1 }], keep: 15 }))
    const history = await clickhouseDashboardService.listVersions("b 1")
    expect(calls[0].url).toBe("/api/dashboard/versions?board=b%201")
    expect(history.versions[0].summary).toBe("Renamed")
  })

  it("posts the board and version to go back and keeps the server's sentence when it refuses", async () => {
    const calls = stubFetch(() =>
      json({ error: 'Cannot go back to version 3: the chart "Hotels" no longer works (x). Nothing was changed.' }, 422),
    )
    const err = await clickhouseDashboardService.revertToVersion("b1", 3).catch((e: unknown) => e)
    expect(calls[0].url).toBe("/api/dashboard/versions/revert")
    expect(calls[0].init.method).toBe("POST")
    expect(sent(calls[0])).toEqual({ board: "b1", version: 3 })
    expect(err).toBeInstanceOf(ServiceError)
    expect((err as ServiceError).code).toBe("invalid_request")
    expect((err as ServiceError).message).toContain("Nothing was changed.")
  })

  it("reads the trash and restores one item", async () => {
    let n = 0
    const calls = stubFetch(() =>
      n++ === 0
        ? json({ items: [{ kind: "board", id: "b1", name: "Old" }], retentionDays: 30 })
        : json({ ok: true, restored: { kind: "board", id: "b1", charts: 2, boardState: "" } }),
    )
    const trash = await clickhouseDashboardService.listTrash()
    expect(trash.retentionDays).toBe(30)
    const restored = await clickhouseDashboardService.restoreFromTrash("board", "b1")
    expect(calls[1].url).toBe("/api/dashboard/trash/restore")
    expect(sent(calls[1])).toEqual({ kind: "board", id: "b1" })
    expect(restored.charts).toBe(2)
  })

  it("opens the personal space with a POST", async () => {
    const calls = stubFetch(() => json({ folder: { id: "fp_1", name: "Personal space of Ann", parentId: "", createdBy: "u", ownerId: "u" } }))
    const folder = await clickhouseDashboardService.ensurePersonalSpace()
    expect(calls[0].url).toBe("/api/dashboard/personal")
    expect(calls[0].init.method).toBe("POST")
    expect(folder.ownerId).toBe("u")
  })

  it("reads shortcuts, tolerating a missing list, and writes favourites and recents", async () => {
    let n = 0
    const calls = stubFetch(() => (n++ === 0 ? json({ favourites: [{ kind: "board", id: "b1", name: "A" }] }) : json({ ok: true })))
    const s = await clickhouseDashboardService.shortcuts()
    expect(s.favourites).toHaveLength(1)
    expect(s.recent).toEqual([])
    await clickhouseDashboardService.setFavourite("chart", "u1", false)
    expect(calls[1].init.method).toBe("PUT")
    expect(sent(calls[1])).toEqual({ kind: "chart", id: "u1", on: false })
    await clickhouseDashboardService.touchRecent("board", "b1")
    expect(calls[2].url).toBe("/api/dashboard/recent")
    expect(sent(calls[2])).toEqual({ kind: "board", id: "b1" })
  })

  it("searches by name with the text encoded", async () => {
    const calls = stubFetch(() => json({ hits: [{ kind: "board", id: "b1", name: "50% off" }] }))
    const hits = await clickhouseDashboardService.searchItems("50% off")
    expect(calls[0].url).toBe("/api/dashboard/search?q=50%25%20off")
    expect(hits[0].name).toBe("50% off")
  })
})
