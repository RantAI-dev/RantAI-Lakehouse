import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import { clickhouseAssetService } from "./assets"

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

describe("clickhouseAssetService.setCertification", () => {
  it("puts the status, note and replacement as JSON to the asset's certification route", async () => {
    const calls = stubFetch(() => json({ ok: true }))
    await clickhouseAssetService.setCertification!("silver.orders", {
      status: "deprecated",
      note: "Superseded",
      replacementAssetId: "silver.orders_v2",
    })
    expect(calls).toHaveLength(1)
    expect(calls[0].url).toBe("/api/catalog/silver.orders/certification")
    expect(calls[0].init.method).toBe("PUT")
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      status: "deprecated",
      note: "Superseded",
      replacementAssetId: "silver.orders_v2",
    })
  })

  it("sends status null to clear the mark", async () => {
    const calls = stubFetch(() => json({ ok: true }))
    await clickhouseAssetService.setCertification!("silver.orders", { status: null })
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ status: null })
  })

  it("throws the API's sentence as an invalid_request on a 400", async () => {
    stubFetch(() => json({ error: "the replacement is itself deprecated" }, 400))
    const err = await clickhouseAssetService
      .setCertification!("silver.orders", { status: "deprecated", replacementAssetId: "x" })
      .catch((e) => e)
    expect(err).toBeInstanceOf(ServiceError)
    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe("the replacement is itself deprecated")
  })

  it("throws permission_denied on a 403", async () => {
    stubFetch(() => json({ error: "permission_denied: governance:write" }, 403))
    const err = await clickhouseAssetService.setCertification!("silver.orders", { status: "certified" }).catch((e) => e)
    expect(err.code).toBe("permission_denied")
  })
})

describe("clickhouseAssetService.getAsset", () => {
  // QA LH-DATA-13-03 / LH-DATA-15-03 F4 (plan D8): the status picks the kind
  // the asset page shows; it used to be "not_found" for every failure.
  it("throws permission_denied on a 403", async () => {
    stubFetch(() => json({ error: "permission_denied: catalog:read" }, 403))
    const err = await clickhouseAssetService.getAsset("silver.orders").catch((e) => e)
    expect(err).toBeInstanceOf(ServiceError)
    expect(err.code).toBe("permission_denied")
    expect(err.status).toBe(403)
    expect(err.message).toBe("You don't have access. This needs the catalog:read permission.")
  })

  it("throws unavailable on a 503 and keeps the API's sentence", async () => {
    stubFetch(() => json({ error: "This table's details could not be read right now. Reference: abc" }, 503))
    const err = await clickhouseAssetService.getAsset("silver.orders").catch((e) => e)
    expect(err.code).toBe("unavailable")
    expect(err.message).toContain("could not be read right now")
  })

  it("throws not_found on a 404", async () => {
    stubFetch(() => json({ error: "Asset not found" }, 404))
    const err = await clickhouseAssetService.getAsset("silver.orders").catch((e) => e)
    expect(err.code).toBe("not_found")
  })

  it("still names the failure when the body is not JSON", async () => {
    stubFetch(() => new Response("bad gateway", { status: 502 }))
    const err = await clickhouseAssetService.getAsset("silver.orders").catch((e) => e)
    expect(err.code).toBe("unavailable")
  })
})

describe("clickhouseAssetService.listAssetsPage", () => {
  // QA LH-DATA-12-09, TC-1724 (WEB-11): the Catalog said "Service unavailable"
  // with Retry for a missing permission.
  it("throws permission_denied with the sentence that names the permission on a 403", async () => {
    stubFetch(() => json({ error: "permission_denied: catalog:read" }, 403))
    const err = await clickhouseAssetService
      .listAssetsPage({ page: 1, pageSize: 50 })
      .catch((e) => e)
    expect(err).toBeInstanceOf(ServiceError)
    expect(err.code).toBe("permission_denied")
    expect(err.status).toBe(403)
    expect(err.message).toBe("You don't have access. This needs the catalog:read permission.")
  })

  it("still calls a 400 a bad request and a 500 an outage", async () => {
    stubFetch(() => json({ error: "unknown field" }, 400))
    const bad = await clickhouseAssetService
      .listAssetsPage({ page: 1, pageSize: 50 })
      .catch((e) => e)
    expect(bad.code).toBe("invalid_request")

    stubFetch(() => json({ error: "The catalog did not answer." }, 500))
    const down = await clickhouseAssetService
      .listAssetsPage({ page: 1, pageSize: 50 })
      .catch((e) => e)
    expect(down.code).toBe("unavailable")
  })
})

describe("clickhouseAssetService.grantAccess", () => {
  it("gives the server's reason for a grant it refuses", async () => {
    stubFetch(() => json({ error: "permission_denied: you cannot grant access to yourself" }, 403))
    const err = await clickhouseAssetService
      .grantAccess("silver.orders", { email: "me@meridian.example", permission: "lineage:read" })
      .catch((e) => e)
    expect(err).toBeInstanceOf(ServiceError)
    expect(err.code).toBe("permission_denied")
    expect(err.message).toBe(
      "You don't have access. Reason given: you cannot grant access to yourself."
    )
  })
})

describe("clickhouseAssetService.searchAssets", () => {
  // QA `LH-DATA-11-02` (`DATA-54`): the box shows eight of the matches and
  // needs the total the server already counts (`DATA-41` D3).
  it("asks for the limit and answers the total", async () => {
    const calls = stubFetch(() => json({ assets: [{ id: "silver.orders" }], total: 23 }))
    const found = await clickhouseAssetService.searchAssets("orders", 8)
    expect(calls[0]?.url).toBe("/api/catalog?q=orders&limit=8")
    expect(found.total).toBe(23)

    stubFetch(() => json({ assets: [{ id: "silver.a" }, { id: "silver.b" }] }))
    expect((await clickhouseAssetService.searchAssets("orders", 8)).total).toBe(2)
  })
})

describe("clickhouseAssetService: dropping a table", () => {
  it("asks for the drop status without a count, and with one only when asked", async () => {
    const calls = stubFetch(() => json({ droppable: true, table: "bronze.orders", keptDays: 7 }))
    await clickhouseAssetService.getDropStatus("orders")
    await clickhouseAssetService.getDropStatus("orders", { withRowCount: true })
    expect(calls.map((c) => c.url)).toEqual([
      "/api/catalog/orders/drop",
      "/api/catalog/orders/drop?withRowCount=true",
    ])
  })

  it("posts the drop and returns what the API says was dropped", async () => {
    const calls = stubFetch(() =>
      json({
        ok: true,
        table: "bronze.orders",
        rowCount: 1234,
        droppedAt: "2026-11-13T08:00:00Z",
        expiresAt: "2026-11-20T08:00:00Z",
      })
    )
    const result = await clickhouseAssetService.dropTable("orders")
    expect(calls).toHaveLength(1)
    expect(calls[0].url).toBe("/api/catalog/orders/drop")
    expect(calls[0].init.method).toBe("POST")
    expect(result.table).toBe("bronze.orders")
    expect(result.rowCount).toBe(1234)
    expect(result.expiresAt).toBe("2026-11-20T08:00:00Z")
  })

  it("throws the API's sentence as an invalid_request on a 409", async () => {
    stubFetch(() => json({ error: "Only a table made by a file upload can be dropped here." }, 409))
    const err = await clickhouseAssetService.dropTable("orders").catch((e) => e)
    expect(err).toBeInstanceOf(ServiceError)
    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe("Only a table made by a file upload can be dropped here.")
  })

  it("throws permission_denied with the permission named on a 403", async () => {
    stubFetch(() => json({ error: "permission_denied: governance:write" }, 403))
    const err = await clickhouseAssetService.dropTable("orders").catch((e) => e)
    expect(err).toBeInstanceOf(ServiceError)
    expect(err.code).toBe("permission_denied")
    expect(err.message).toBe("You don't have access. This needs the governance:write permission.")
  })

  it("reads the dropped tables, and an answer without a list as none", async () => {
    const calls = stubFetch(() =>
      json({
        tables: [
          {
            assetId: "orders",
            table: "bronze.orders",
            droppedBy: "Ada Lovelace",
            rowCount: null,
            droppedAt: "2026-11-13T08:00:00Z",
            expiresAt: "2026-11-20T08:00:00Z",
          },
        ],
      })
    )
    const rows = await clickhouseAssetService.listDroppedTables()
    expect(calls[0].url).toBe("/api/catalog/dropped/tables")
    expect(rows).toHaveLength(1)
    expect(rows[0].table).toBe("bronze.orders")
    expect(rows[0].rowCount).toBeNull()

    stubFetch(() => json({ supported: false, reason: "not the catalog's tenant" }))
    expect(await clickhouseAssetService.listDroppedTables()).toEqual([])
  })
})
