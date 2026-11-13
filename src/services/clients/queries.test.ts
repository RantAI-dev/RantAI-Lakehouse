import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
import type { QueryParamDef, SavedQueryAccessList } from "../contracts/queries"
import { clickhouseQueryService } from "./queries"

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

const RAN_SQL = {
  answer: "There are 3 rows.",
  toolTrace: [
    {
      tool: "run_sql",
      ok: true,
      args: { sql: "SELECT 1 AS n" },
      result: { columns: [{ name: "n" }], rows: [{ n: 1 }], rowCount: 1 },
    },
  ],
}

describe("clickhouseQueryService.askQuestion request", () => {
  it("posts the question to /api/ai/chat as JSON with the read-only tool allowlist and no mode or stream", async () => {
    const calls = stubFetch(() => json(RAN_SQL))
    await clickhouseQueryService.askQuestion("how many orders?")

    expect(calls).toHaveLength(1)
    expect(calls[0]!.url).toBe("/api/ai/chat")
    expect(calls[0]!.init.method).toBe("POST")
    expect((calls[0]!.init.headers as Headers).get("Content-Type")).toBe("application/json")
    // Exact match: any extra key, such as `mode` or `stream`, fails the test.
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      messages: [{ role: "user", content: "how many orders?" }],
      tools: ["run_sql", "list_datasets", "describe_dataset", "lakehouse_overview"],
    })
  })
})

describe("clickhouseQueryService.askQuestion failures", () => {
  it("raises unavailable with the body detail when the chat answers with an error status", async () => {
    stubFetch(() => json({ detail: "model is loading", error: "chat_unavailable" }, 503))
    const err = await rejection(clickhouseQueryService.askQuestion("q"))
    expect(err.code).toBe("unavailable")
    expect(err.message).toBe("model is loading")
  })

  it("raises unavailable with the body error when the error body has no detail", async () => {
    stubFetch(() => json({ error: "chat_unavailable" }, 500))
    const err = await rejection(clickhouseQueryService.askQuestion("q"))
    expect(err.code).toBe("unavailable")
    expect(err.message).toBe("chat_unavailable")
  })

  it("raises unavailable with the fallback text when the error body is not JSON", async () => {
    stubFetch(() => new Response("<html>bad gateway</html>", { status: 502 }))
    const err = await rejection(clickhouseQueryService.askQuestion("q"))
    expect(err.code).toBe("unavailable")
    expect(err.message).toBe("AI Copilot could not answer. Try again.")
  })

  it("raises unavailable instead of an empty answer when a 200 response body is not JSON", async () => {
    stubFetch(() => new Response("<html>proxy page</html>", { status: 200 }))
    const err = await rejection(clickhouseQueryService.askQuestion("q"))
    expect(err.code).toBe("unavailable")
    expect(err.message).toBe("AI Copilot could not answer. Try again.")
  })

  it("raises unavailable instead of an empty answer when a 200 response body is JSON but not an object", async () => {
    stubFetch(() => json(["answer"]))
    const err = await rejection(clickhouseQueryService.askQuestion("q"))
    expect(err.code).toBe("unavailable")
    expect(err.message).toBe("AI Copilot could not answer. Try again.")
  })
})

describe("clickhouseQueryService.generateSql", () => {
  it("returns the last successful query with the answer as its explanation", async () => {
    stubFetch(() => json(RAN_SQL))
    await expect(clickhouseQueryService.generateSql("how many orders?")).resolves.toEqual({
      sql: "SELECT 1 AS n",
      explanation: "There are 3 rows.",
    })
  })

  it("raises unavailable with the answer when no run_sql in the trace succeeded", async () => {
    stubFetch(() =>
      json({
        answer: "I could not find a table for that.",
        toolTrace: [
          { tool: "list_datasets", ok: true, args: {}, result: {} },
          { tool: "run_sql", ok: false, args: { sql: "SELECT nope" }, result: { error: "unknown column" } },
        ],
      }),
    )
    const err = await rejection(clickhouseQueryService.generateSql("q"))
    expect(err.code).toBe("unavailable")
    expect(err.message).toBe("I could not find a table for that.")
  })

  it("passes the error through when the chat answers with an error status", async () => {
    stubFetch(() => json({ error: "chat_unavailable" }, 500))
    const err = await rejection(clickhouseQueryService.generateSql("q"))
    expect(err.code).toBe("unavailable")
    expect(err.message).toBe("chat_unavailable")
  })
})

describe("clickhouseQueryService.explainSql request", () => {
  it("posts the explain sentence, the engine and the statement as one user message with propose_sql as the only tool", async () => {
    const calls = stubFetch(() => json({ answer: "It counts rows.", toolTrace: [] }))
    await expect(clickhouseQueryService.explainSql("SELECT count() FROM t", "clickhouse")).resolves.toBe(
      "It counts rows.",
    )

    expect(calls).toHaveLength(1)
    expect(calls[0]!.url).toBe("/api/ai/chat")
    expect(calls[0]!.init.method).toBe("POST")
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      messages: [
        {
          role: "user",
          content: "Explain what this query does.\n\nEngine: clickhouse\nSQL:\nSELECT count() FROM t",
        },
      ],
      tools: ["propose_sql"],
    })
  })
})

describe("clickhouseQueryService.proposeFix request", () => {
  it("posts the fix sentence, the engine, the statement and the error as one user message with propose_sql as the only tool", async () => {
    const calls = stubFetch(() => json({ answer: "No fix.", toolTrace: [] }))
    await clickhouseQueryService.proposeFix("SELECT nope FROM t", "clickhouse", "Unknown identifier: nope")

    expect(calls).toHaveLength(1)
    expect(calls[0]!.url).toBe("/api/ai/chat")
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      messages: [
        {
          role: "user",
          content:
            "Why does this query fail? Fix it.\n\nEngine: clickhouse\nSQL:\nSELECT nope FROM t\nError:\nUnknown identifier: nope",
        },
      ],
      tools: ["propose_sql"],
    })
  })

  it("returns the sql and reason of the proposal the chat accepted", async () => {
    stubFetch(() =>
      json({
        answer: "The column is named amount.",
        toolTrace: [
          {
            tool: "propose_sql",
            ok: true,
            args: { sql: "SELECT amount FROM t", reason: "The column is named amount." },
            result: {
              proposed: true,
              sql: "SELECT amount FROM t",
              reason: "The column is named amount.",
              engine: "clickhouse",
            },
          },
        ],
      }),
    )
    await expect(
      clickhouseQueryService.proposeFix("SELECT amont FROM t", "clickhouse", "unknown column"),
    ).resolves.toEqual({ sql: "SELECT amount FROM t", reason: "The column is named amount." })
  })

  it("returns the last successful proposal when an earlier one failed the check", async () => {
    stubFetch(() =>
      json({
        answer: "The column is named amount.",
        toolTrace: [
          {
            tool: "propose_sql",
            ok: false,
            args: { sql: "SELECT amont2 FROM t", reason: "guess" },
            result: { error: "Unknown identifier: amont2" },
          },
          {
            tool: "propose_sql",
            ok: true,
            args: { sql: "SELECT amount FROM t", reason: "The column is named amount." },
            result: {
              proposed: true,
              sql: "SELECT amount FROM t",
              reason: "The column is named amount.",
              engine: "clickhouse",
            },
          },
        ],
      }),
    )
    await expect(
      clickhouseQueryService.proposeFix("SELECT amont FROM t", "clickhouse", "unknown column"),
    ).resolves.toEqual({ sql: "SELECT amount FROM t", reason: "The column is named amount." })
  })

  it("skips a refused proposal that comes after an accepted one, so only a checked statement is returned", async () => {
    stubFetch(() =>
      json({
        answer: "The column is named amount.",
        toolTrace: [
          {
            tool: "propose_sql",
            ok: true,
            args: { sql: "SELECT amount FROM t", reason: "The column is named amount." },
            result: {
              proposed: true,
              sql: "SELECT amount FROM t",
              reason: "The column is named amount.",
              engine: "clickhouse",
            },
          },
          {
            tool: "propose_sql",
            ok: false,
            args: { sql: "SELECT amont2 FROM t", reason: "guess" },
            result: { error: "Unknown identifier: amont2" },
          },
        ],
      }),
    )
    await expect(
      clickhouseQueryService.proposeFix("SELECT amont FROM t", "clickhouse", "unknown column"),
    ).resolves.toEqual({ sql: "SELECT amount FROM t", reason: "The column is named amount." })
  })

  it("returns the answer alone when the chat proposed no fix", async () => {
    stubFetch(() =>
      json({
        answer: "I need the table's columns to fix this.",
        toolTrace: [
          {
            tool: "propose_sql",
            ok: false,
            args: { sql: "SELECT x", reason: "guess" },
            result: { error: "Unknown identifier: x" },
          },
        ],
      }),
    )
    await expect(
      clickhouseQueryService.proposeFix("SELECT x", "clickhouse", "Unknown identifier: x"),
    ).resolves.toEqual({ answer: "I need the table's columns to fix this." })
  })

  it("refuses a statement of more than 20,000 characters before any call, counting code points", async () => {
    const calls = stubFetch(() => json({ answer: "", toolTrace: [] }))
    const long = "é".repeat(20_001)
    const err = await rejection(clickhouseQueryService.proposeFix(long, "clickhouse", "err"))
    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe("This query is too long for AI Copilot to fix.")
    expect(calls).toHaveLength(0)
  })

  it("accepts a statement of exactly 20,000 characters", async () => {
    const calls = stubFetch(() => json({ answer: "", toolTrace: [] }))
    const exact = "a".repeat(20_000)
    await clickhouseQueryService.proposeFix(exact, "clickhouse", "err")
    expect(calls).toHaveLength(1)
  })
})

describe("Stop (BI-5 plan D5)", () => {
  it("sends the run id as X-Run-Id with the run", async () => {
    const calls = stubFetch(() => json({ id: "r", columns: [], rows: [] }))
    await clickhouseQueryService.run("SELECT 1", { engine: "clickhouse", runId: "7f0c0f0e-3c7e-4a3e-9d4f-0a1b2c3d4e5f" })
    expect(calls[0].url).toBe("/api/query/run")
    expect(new Headers(calls[0].init.headers).get("X-Run-Id")).toBe("7f0c0f0e-3c7e-4a3e-9d4f-0a1b2c3d4e5f")
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ sql: "SELECT 1", engine: "clickhouse" })
  })

  it("sends no X-Run-Id when the caller has none", async () => {
    const calls = stubFetch(() => json({ id: "r", columns: [], rows: [] }))
    await clickhouseQueryService.run("SELECT 1", { engine: "clickhouse" })
    expect(new Headers(calls[0].init.headers).has("X-Run-Id")).toBe(false)
  })

  it("posts the run id to the cancel route and returns how many queries it stopped", async () => {
    const calls = stubFetch(() => json({ stopped: 2 }))
    expect(await clickhouseQueryService.cancel("7f0c0f0e-3c7e-4a3e-9d4f-0a1b2c3d4e5f")).toEqual({ stopped: 2 })
    expect(calls[0].url).toBe("/api/query/cancel")
    expect(calls[0].init.method).toBe("POST")
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ runId: "7f0c0f0e-3c7e-4a3e-9d4f-0a1b2c3d4e5f" })
  })

  it("reports a missing cancel route as not found", async () => {
    stubFetch(() => json({ error: "Not found" }, 404))
    expect((await rejection(clickhouseQueryService.cancel("7f0c0f0e-3c7e-4a3e-9d4f-0a1b2c3d4e5f"))).code).toBe("not_found")
  })
})

describe("the query download", () => {
  const RUN_ID = "7f0c0f0e-3c7e-4a3e-9d4f-0a1b2c3d4e5f"

  it("asks for the file of one run, with the format and the run the run had", async () => {
    const calls = stubFetch(() => new Response("a,b\r\n", { status: 200 }))
    await clickhouseQueryService.download("q-1", "csv", RUN_ID)
    expect(calls[0].url).toBe(`/api/query/run/q-1/download?format=csv&runId=${RUN_ID}`)
  })

  it("reads the name, the rows, the cut and the limit that cut the file from the response headers", async () => {
    stubFetch(() =>
      new Response("a,b\r\n", {
        status: 200,
        headers: {
          "content-disposition": 'attachment; filename="q-1-first-100-rows.csv"',
          "x-export-rows": "100",
          "x-export-cut": "true",
          "x-export-cut-by": "rows",
        },
      }),
    )
    const out = await clickhouseQueryService.download("q-1", "csv", RUN_ID)
    expect(out.fileName).toBe("q-1-first-100-rows.csv")
    expect(out.rows).toBe(100)
    expect(out.cut).toBe(true)
    expect(out.cutBy).toBe("rows")
  })

  it("counts no rows and claims no cut when the server says neither", async () => {
    stubFetch(() =>
      new Response("x", { status: 200, headers: { "x-export-cut": "false" } }),
    )
    const out = await clickhouseQueryService.download("q-1", "parquet", RUN_ID)
    expect(out.rows).toBeNull()
    expect(out.cut).toBe(false)
    expect(out.cutBy).toBeNull()
    // The download asked for a Parquet file, so the fallback name is a
    // Parquet file: a Parquet file named `query.csv` will not open.
    expect(out.fileName).toBe("query.parquet")
  })

  it("names the file after the format it asked for when the server names no file", async () => {
    for (const format of ["csv", "tsv", "xlsx", "parquet"] as const) {
      stubFetch(() => new Response("x", { status: 200 }))
      const out = await clickhouseQueryService.download("q-1", format, RUN_ID)
      expect(out.fileName).toBe(`query.${format}`)
    }
  })

  it("throws the server's own sentence when the download is refused", async () => {
    stubFetch(() =>
      json({ error: "permission_denied: Downloads are switched off for your role." }, 403),
    )
    await expect(
      clickhouseQueryService.download("q-1", "csv", RUN_ID),
    ).rejects.toThrow("Downloads are switched off for your role.")
  })

  it("rejects a refused download with the ServiceError the status implies", async () => {
    stubFetch(() =>
      json({ error: "permission_denied: Downloads are switched off for your role." }, 403),
    )
    const err = await rejection(clickhouseQueryService.download("q-1", "csv", RUN_ID))
    expect(err.code).toBe("permission_denied")
    expect(err.message).toBe("Downloads are switched off for your role.")
  })
})

describe("clickhouseQueryService.listHistoryPage", () => {
  const page = (runs: unknown[] = [], total = 0, number = 1) => ({
    runs,
    total,
    page: number,
    pageSize: 50,
  })

  it("asks /api/query/history with no query string when no filter is given", async () => {
    const calls = stubFetch(() => json(page()))
    const out = await clickhouseQueryService.listHistoryPage()
    expect(out).toEqual({ runs: [], total: 0, page: 1, pageSize: 50 })
    expect(calls[0]!.url).toBe("/api/query/history")
  })

  it("appends the filter's query string, encoded, after a single ?", async () => {
    const calls = stubFetch(() => json(page()))
    await clickhouseQueryService.listHistoryPage({
      status: "stopped",
      table: "ord%",
      minDurationMs: 0,
      page: 3,
    })
    expect(calls[0]!.url).toBe(
      "/api/query/history?status=stopped&minDurationMs=0&table=ord%25&page=3",
    )
  })

  it("adds no query string for a filter of empty strings", async () => {
    const calls = stubFetch(() => json(page()))
    await clickhouseQueryService.listHistoryPage({ table: "" })
    expect(calls[0]!.url).toBe("/api/query/history")
  })

  it("hands back the envelope as the server sent it", async () => {
    stubFetch(() => json(page([{ id: "q-1" }], 120, 2)))
    const out = await clickhouseQueryService.listHistoryPage({ page: 2 })
    expect(out.total).toBe(120)
    expect(out.page).toBe(2)
    expect(out.pageSize).toBe(50)
    expect(out.runs).toHaveLength(1)
  })
})

describe("clickhouseQueryService.listHistory", () => {
  it("returns the first page's runs, not the envelope", async () => {
    stubFetch(() =>
      json({ runs: [{ id: "q-1" }, { id: "q-2" }], total: 2, page: 1, pageSize: 50 }),
    )
    const rows = await clickhouseQueryService.listHistory()
    expect(rows.map((r) => r.id)).toEqual(["q-1", "q-2"])
  })
})

/**
 * The saved-query console (`QS-3`). Every route the console uses is
 * here: the list and its filters, one query, the trash, the versions,
 * the folders and the access lists. The tests pin the URL, the body and
 * the refusal of each one, because the console shows the server's own
 * sentence and never a sentence of its own.
 */
describe("the saved-query console client (QS-3)", () => {
  const SAVED_QUERY = {
    id: "q-1",
    title: "Orders by day",
    sql: "SELECT count(*) FROM silver.orders",
    owner: "Rina Wijaya",
    updatedAt: "2026-11-04T10:00:00.000Z",
    tags: ["daily"],
    folderId: null,
    ownerId: "u-1",
    level: "manage",
    parameters: [],
  }

  const FOLDER = {
    id: "f-1",
    parentId: null,
    name: "Reports",
    isPersonal: false,
    ownerId: "u-1",
    level: "manage",
  }

  const ACCESS_LIST: SavedQueryAccessList = {
    itemType: "query",
    itemId: "q-1",
    yourLevel: "manage",
    rules: [
      {
        id: "rule-1",
        userId: "u-2",
        roleId: null,
        granteeName: "Dewi Lestari",
        granteeEmail: "dewi@example.com",
        level: "edit",
        grantedBy: "u-1",
        grantedAt: "2026-11-04T10:00:00Z",
      },
    ],
    inherited: [
      {
        folderId: "f-1",
        folderName: "Reports",
        userId: null,
        roleId: "r-1",
        granteeName: "Finance",
        granteeEmail: null,
        level: "view",
      },
    ],
  }

  it("lists the saved queries the caller may see", async () => {
    const calls = stubFetch(() => json([SAVED_QUERY]))
    const rows = await clickhouseQueryService.listSaved()
    expect(rows).toEqual([SAVED_QUERY])
    expect(calls).toHaveLength(1)
    expect(calls[0]!.url).toBe("/api/query/saved")
  })

  it("asks the server for the queries of one folder and the text typed in the search box", async () => {
    const calls = stubFetch(() => json([SAVED_QUERY]))
    await clickhouseQueryService.listSavedFiltered({ q: "orders", folder: "f-1" })
    expect(calls[0]!.url).toBe("/api/query/saved?q=orders&folder=f-1")
  })

  it("asks for the shared space with the word the server reads as the root", async () => {
    const calls = stubFetch(() => json([]))
    await clickhouseQueryService.listSavedFiltered({ folder: "root" })
    expect(calls[0]!.url).toBe("/api/query/saved?folder=root")
  })

  it("encodes the text typed in the search box", async () => {
    const calls = stubFetch(() => json([]))
    await clickhouseQueryService.listSavedFiltered({ q: "last 30 days" })
    expect(calls[0]!.url).toBe("/api/query/saved?q=last+30+days")
  })

  it("adds no query string for a filter with nothing in it", async () => {
    const calls = stubFetch(() => json([]))
    await clickhouseQueryService.listSavedFiltered({})
    expect(calls[0]!.url).toBe("/api/query/saved")
  })

  it("reads one saved query", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY))
    const query = await clickhouseQueryService.getSaved("q-1")
    expect(query).toEqual(SAVED_QUERY)
    expect(calls[0]!.url).toBe("/api/query/saved/q-1")
  })

  it("escapes the id of the query it reads", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY))
    await clickhouseQueryService.getSaved("q 1")
    expect(calls[0]!.url).toBe("/api/query/saved/q%201")
  })

  it("saves a query with its title, its statement and its tags", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY, 201))
    const saved = await clickhouseQueryService.saveQuery({
      title: "Orders by day",
      sql: "SELECT count(*) FROM silver.orders",
      tags: ["daily"],
    })
    expect(saved).toEqual(SAVED_QUERY)
    expect(calls[0]!.url).toBe("/api/query/saved")
    expect(calls[0]!.init.method).toBe("POST")
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      title: "Orders by day",
      sql: "SELECT count(*) FROM silver.orders",
      tags: ["daily"],
    })
  })

  it("saves a query in the caller's own space when the body names no folder", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY, 201))
    await clickhouseQueryService.saveQuery({
      title: "Orders by day",
      sql: "SELECT 1",
      tags: [],
      folderId: undefined,
    })
    const body = JSON.parse(String(calls[0]!.init.body)) as Record<string, unknown>
    expect("folderId" in body).toBe(false)
  })

  it("saves a query in the shared space", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY, 201))
    await clickhouseQueryService.saveQuery({
      title: "Orders by day",
      sql: "SELECT 1",
      tags: [],
      folderId: null,
    })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      title: "Orders by day",
      sql: "SELECT 1",
      tags: [],
      folderId: null,
    })
  })

  it("saves a query in a folder", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY, 201))
    await clickhouseQueryService.saveQuery({
      title: "Orders by day",
      sql: "SELECT 1",
      tags: [],
      folderId: "f-1",
    })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      title: "Orders by day",
      sql: "SELECT 1",
      tags: [],
      folderId: "f-1",
    })
  })

  it("changes the title, the statement and the tags of a query", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY))
    const changed = await clickhouseQueryService.updateSaved("q-1", {
      title: "Orders by week",
      sql: "SELECT 2",
      tags: [],
    })
    expect(changed).toEqual(SAVED_QUERY)
    expect(calls[0]!.url).toBe("/api/query/saved/q-1")
    expect(calls[0]!.init.method).toBe("PATCH")
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      title: "Orders by week",
      sql: "SELECT 2",
      tags: [],
    })
  })

  it("leaves a query where it is when the change names no folder", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY))
    await clickhouseQueryService.updateSaved("q-1", { title: "Orders by week" })
    const body = JSON.parse(String(calls[0]!.init.body)) as Record<string, unknown>
    expect("folderId" in body).toBe(false)
  })

  it("moves a query to the shared space", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY))
    await clickhouseQueryService.updateSaved("q-1", { folderId: null })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({ folderId: null })
  })

  it("moves a query into a folder", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY))
    await clickhouseQueryService.updateSaved("q-1", { folderId: "f-1" })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({ folderId: "f-1" })
  })

  it("takes a query to the trash and answers with nothing", async () => {
    const calls = stubFetch(() => new Response(null, { status: 204 }))
    await expect(clickhouseQueryService.deleteSaved("q-1")).resolves.toBeUndefined()
    expect(calls[0]!.url).toBe("/api/query/saved/q-1")
    expect(calls[0]!.init.method).toBe("DELETE")
  })

  it("copies a query into the caller's own space", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY, 201))
    const copy = await clickhouseQueryService.duplicateSaved("q-1")
    expect(copy).toEqual(SAVED_QUERY)
    expect(calls[0]!.url).toBe("/api/query/saved/q-1/duplicate")
    expect(calls[0]!.init.method).toBe("POST")
  })

  it("brings a trashed query back", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY))
    const restored = await clickhouseQueryService.restoreSaved("q-1")
    expect(restored).toEqual(SAVED_QUERY)
    expect(calls[0]!.url).toBe("/api/query/saved/q-1/restore")
    expect(calls[0]!.init.method).toBe("POST")
  })

  it("lists the queries in the trash with the days they have left", async () => {
    const calls = stubFetch(() =>
      json([
        {
          id: "q-9",
          title: "Old orders",
          folderId: "f-1",
          deletedAt: "2026-11-04T10:00:00.000Z",
          daysLeft: 29,
        },
      ]),
    )
    const rows = await clickhouseQueryService.listSavedTrash()
    expect(rows).toEqual([
      {
        id: "q-9",
        title: "Old orders",
        folderId: "f-1",
        deletedAt: "2026-11-04T10:00:00.000Z",
        daysLeft: 29,
      },
    ])
    expect(calls[0]!.url).toBe("/api/query/saved-trash")
  })

  it("lists the versions of a query, newest first", async () => {
    const calls = stubFetch(() =>
      json([
        {
          version: 2,
          title: "Orders by day",
          sql: "SELECT 1",
          authorId: "u-1",
          authorName: "Rina Wijaya",
          label: "Restored version 1",
          createdAt: "2026-11-04T10:00:00.000Z",
          parameters: [],
        },
      ]),
    )
    const versions = await clickhouseQueryService.listSavedVersions("q-1")
    expect(versions).toEqual([
      {
        version: 2,
        title: "Orders by day",
        sql: "SELECT 1",
        authorId: "u-1",
        authorName: "Rina Wijaya",
        label: "Restored version 1",
        createdAt: "2026-11-04T10:00:00.000Z",
        parameters: [],
      },
    ])
    expect(calls[0]!.url).toBe("/api/query/saved/q-1/versions")
  })

  it("goes back to an earlier version of a query", async () => {
    const calls = stubFetch(() => json(SAVED_QUERY))
    const reverted = await clickhouseQueryService.revertSavedVersion("q-1", 1)
    expect(reverted).toEqual(SAVED_QUERY)
    expect(calls[0]!.url).toBe("/api/query/saved/q-1/versions/1/revert")
    expect(calls[0]!.init.method).toBe("POST")
  })

  it("lists the folders the caller may see", async () => {
    const calls = stubFetch(() => json([FOLDER]))
    const folders = await clickhouseQueryService.listFolders()
    expect(folders).toEqual([FOLDER])
    expect(calls[0]!.url).toBe("/api/query/folders")
  })

  it("makes a folder in the root", async () => {
    const calls = stubFetch(() => json(FOLDER, 201))
    const folder = await clickhouseQueryService.createFolder({ name: "Reports" })
    expect(folder).toEqual(FOLDER)
    expect(calls[0]!.url).toBe("/api/query/folders")
    expect(calls[0]!.init.method).toBe("POST")
    const body = JSON.parse(String(calls[0]!.init.body)) as Record<string, unknown>
    expect("parentId" in body).toBe(false)
  })

  it("makes a folder inside another folder", async () => {
    const calls = stubFetch(() => json(FOLDER, 201))
    await clickhouseQueryService.createFolder({ name: "Reports", parentId: "f-0" })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      name: "Reports",
      parentId: "f-0",
    })
  })

  it("renames a folder", async () => {
    const calls = stubFetch(() => json(FOLDER))
    const folder = await clickhouseQueryService.updateFolder("f-1", { name: "Weekly reports" })
    expect(folder).toEqual(FOLDER)
    expect(calls[0]!.url).toBe("/api/query/folders/f-1")
    expect(calls[0]!.init.method).toBe("PATCH")
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({ name: "Weekly reports" })
  })

  it("moves a folder to the root", async () => {
    const calls = stubFetch(() => json(FOLDER))
    await clickhouseQueryService.updateFolder("f-1", { parentId: null })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({ parentId: null })
  })

  it("leaves a folder where it is when the change names no parent", async () => {
    const calls = stubFetch(() => json(FOLDER))
    await clickhouseQueryService.updateFolder("f-1", { name: "Weekly reports" })
    const body = JSON.parse(String(calls[0]!.init.body)) as Record<string, unknown>
    expect("parentId" in body).toBe(false)
  })

  it("deletes a folder and answers with nothing", async () => {
    const calls = stubFetch(() => new Response(null, { status: 204 }))
    await expect(clickhouseQueryService.deleteFolder("f-1")).resolves.toBeUndefined()
    expect(calls[0]!.url).toBe("/api/query/folders/f-1")
    expect(calls[0]!.init.method).toBe("DELETE")
  })

  it("reads who has what on a query", async () => {
    const calls = stubFetch(() => json(ACCESS_LIST))
    const access = await clickhouseQueryService.getSavedAccess("q-1")
    expect(access).toEqual(ACCESS_LIST)
    expect(calls[0]!.url).toBe("/api/query/saved/q-1/access")
  })

  it("replaces the rules on a query", async () => {
    const calls = stubFetch(() => json(ACCESS_LIST))
    const access = await clickhouseQueryService.putSavedAccess("q-1", {
      rules: [{ userId: "u-2", level: "edit" }],
    })
    expect(access).toEqual(ACCESS_LIST)
    expect(calls[0]!.url).toBe("/api/query/saved/q-1/access")
    expect(calls[0]!.init.method).toBe("PUT")
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      rules: [{ userId: "u-2", level: "edit" }],
    })
  })

  it("reads who has what on a folder", async () => {
    const calls = stubFetch(() => json({ ...ACCESS_LIST, itemType: "folder", itemId: "f-1" }))
    const access = await clickhouseQueryService.getFolderAccess("f-1")
    expect(access.itemType).toBe("folder")
    expect(calls[0]!.url).toBe("/api/query/folders/f-1/access")
  })

  it("replaces the rules on a folder", async () => {
    const calls = stubFetch(() => json({ ...ACCESS_LIST, itemType: "folder", itemId: "f-1" }))
    await clickhouseQueryService.putFolderAccess("f-1", {
      rules: [{ roleId: "r-1", level: "view" }],
    })
    expect(calls[0]!.url).toBe("/api/query/folders/f-1/access")
    expect(calls[0]!.init.method).toBe("PUT")
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      rules: [{ roleId: "r-1", level: "view" }],
    })
  })

  it("searches the people and the roles the caller may share with", async () => {
    const calls = stubFetch(() =>
      json({
        people: [{ id: "u-2", name: "Reza Hakim", email: "reza@example.com" }],
        roles: [{ id: "r-1", name: "Analyst" }],
      }),
    )
    const grantees = await clickhouseQueryService.findGrantees("reza")
    expect(grantees).toEqual({
      people: [{ id: "u-2", name: "Reza Hakim", email: "reza@example.com" }],
      roles: [{ id: "r-1", name: "Analyst" }],
    })
    expect(calls[0]!.url).toBe("/api/query/grantees?q=reza")
  })

  it("refuses a move with the server's own sentence", async () => {
    stubFetch(() =>
      json(
        {
          error:
            "permission_denied: Only someone who manages this can move it.",
        },
        403,
      ),
    )
    const err = await rejection(clickhouseQueryService.updateSaved("q-1", { folderId: "f-2" }))
    expect(err.code).toBe("permission_denied")
    expect(err.message).toBe(
      "Only someone who manages this can move it.",
    )
  })

  it("refuses a change of a query the caller may not change with the server's own sentence", async () => {
    stubFetch(() =>
      json(
        { error: "permission_denied: You do not have permission to change this saved query." },
        403,
      ),
    )
    const err = await rejection(clickhouseQueryService.updateSaved("q-1", { title: "New" }))
    expect(err.code).toBe("permission_denied")
    expect(err.message).toBe("You do not have permission to change this saved query.")
  })

  it("reports a query the caller may not see as not found", async () => {
    stubFetch(() => json({ error: "That saved query could not be found." }, 404))
    const err = await rejection(clickhouseQueryService.getSaved("q-9"))
    expect(err.code).toBe("not_found")
    expect(err.message).toBe("That saved query could not be found.")
  })

  it("reports a body of another shape as an invalid request", async () => {
    stubFetch(() =>
      json({ error: "Body must be JSON {rules: [{userId?, roleId?, level}]}." }, 400),
    )
    const err = await rejection(
      clickhouseQueryService.putSavedAccess("q-1", { rules: [{ userId: "u-2", level: "edit" }] }),
    )
    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe("Body must be JSON {rules: [{userId?, roleId?, level}]}.")
  })

  it("reports a name a sibling already carries as an invalid request", async () => {
    stubFetch(() => json({ error: "A folder with that name already exists here." }, 409))
    const err = await rejection(clickhouseQueryService.createFolder({ name: "Reports" }))
    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe("A folder with that name already exists here.")
  })

  it("reports a folder that still holds queries as an invalid request", async () => {
    stubFetch(() =>
      json(
        {
          error:
            "This folder still holds queries, some of them in the trash. Move or restore them first.",
        },
        409,
      ),
    )
    const err = await rejection(clickhouseQueryService.deleteFolder("f-1"))
    expect(err.code).toBe("invalid_request")
    expect(err.message).toBe(
      "This folder still holds queries, some of them in the trash. Move or restore them first.",
    )
  })
})

describe("query parameters on the wire (QS-5)", () => {
  const DEFINITIONS: QueryParamDef[] = [
    { name: "region", type: "text", default: "EU" },
    {
      name: "tier",
      type: "dropdown",
      default: "gold",
      options: [
        { value: "gold", label: "Gold" },
        { value: "silver", label: "Silver" },
      ],
    },
  ]
  const VALUES = { region: "US", span: { start: "2026-11-01", end: "2026-11-07" }, ids: ["a", "b"] }
  const SQL = "SELECT {{region}}"

  it("posts exactly {sql, engine} to run when the caller has no parameters and no definitions", async () => {
    const calls = stubFetch(() => json({ id: "r", columns: [], rows: [] }))
    await clickhouseQueryService.run("SELECT 1", { engine: "trino" })
    expect(calls[0]!.init.body).toBe('{"sql":"SELECT 1","engine":"trino"}')
  })

  it("posts the values and the definitions to run when the caller gives them", async () => {
    const calls = stubFetch(() => json({ id: "r", columns: [], rows: [] }))
    await clickhouseQueryService.run(SQL, {
      engine: "clickhouse",
      parameters: VALUES,
      definitions: DEFINITIONS,
    })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      sql: SQL,
      engine: "clickhouse",
      parameters: VALUES,
      definitions: DEFINITIONS,
    })
  })

  it("posts the values to run without definitions when the caller has none", async () => {
    const calls = stubFetch(() => json({ id: "r", columns: [], rows: [] }))
    await clickhouseQueryService.run(SQL, { engine: "clickhouse", parameters: { region: "US" } })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      sql: SQL,
      engine: "clickhouse",
      parameters: { region: "US" },
    })
  })

  it("posts exactly {sql} to estimate when the caller has no parameters and no definitions", async () => {
    const calls = stubFetch(() => json({ estimatedBytes: null }))
    await clickhouseQueryService.estimate("SELECT 1")
    expect(calls[0]!.url).toBe("/api/query/estimate")
    expect(calls[0]!.init.body).toBe('{"sql":"SELECT 1"}')
  })

  it("posts the values and the definitions to estimate when the caller gives them", async () => {
    const calls = stubFetch(() => json({ estimatedBytes: null }))
    await clickhouseQueryService.estimate(SQL, { parameters: VALUES, definitions: DEFINITIONS })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      sql: SQL,
      parameters: VALUES,
      definitions: DEFINITIONS,
    })
  })


  it("stops an estimate through the signal that follows the parameters", async () => {
    const calls = stubFetch(() => json({ estimatedBytes: null }))
    const controller = new AbortController()
    await clickhouseQueryService.estimate("SELECT 1", undefined, controller.signal)
    expect(calls[0]!.init.signal).toBe(controller.signal)
  })

  it("saves a query with its definitions", async () => {
    const calls = stubFetch(() => json({}, 201))
    await clickhouseQueryService.saveQuery({
      title: "By region",
      sql: SQL,
      tags: [],
      parameters: DEFINITIONS,
    })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({
      title: "By region",
      sql: SQL,
      tags: [],
      parameters: DEFINITIONS,
    })
  })

  it("changes the definitions of a query, and an empty list clears them", async () => {
    const calls = stubFetch(() => json({}))
    await clickhouseQueryService.updateSaved("q-1", { parameters: DEFINITIONS })
    await clickhouseQueryService.updateSaved("q-1", { parameters: [] })
    expect(JSON.parse(String(calls[0]!.init.body))).toStrictEqual({ parameters: DEFINITIONS })
    expect(JSON.parse(String(calls[1]!.init.body))).toStrictEqual({ parameters: [] })
  })

  it("leaves the definitions of a query alone when the change names none", async () => {
    const calls = stubFetch(() => json({}))
    await clickhouseQueryService.updateSaved("q-1", { title: "By region" })
    expect(calls[0]!.init.body).toBe('{"title":"By region"}')
  })

  it("hands back the definitions a saved query and its versions carry", async () => {
    stubFetch(() => json({ id: "q-1", parameters: DEFINITIONS }))
    const saved = await clickhouseQueryService.getSaved("q-1")
    expect(saved.parameters).toEqual(DEFINITIONS)
    stubFetch(() => json([{ version: 1, parameters: DEFINITIONS }]))
    const versions = await clickhouseQueryService.listSavedVersions("q-1")
    expect(versions[0]!.parameters).toEqual(DEFINITIONS)
  })
})

describe("refusals carry their status (WEB-12)", () => {
  it("a refused saved query rejects with permission_denied, the status and the sentence", async () => {
    stubFetch(() => json({ error: "permission_denied: query:read" }, 403))
    await expect(clickhouseQueryService.getSaved("q-9")).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the query:read permission.",
    })
  })

  it("a refused delete rejects with permission_denied, the status and the sentence", async () => {
    stubFetch(() => json({ error: "permission_denied: query:write" }, 403))
    await expect(clickhouseQueryService.deleteSaved("q-1")).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the query:write permission.",
    })
  })

  it("a refused download keeps the server's sentence and gains its status", async () => {
    stubFetch(() =>
      json({ error: "permission_denied: Downloads are switched off for your role." }, 403),
    )
    await expect(
      clickhouseQueryService.download("q-1", "csv", "7f0c0f0e-3c7e-4a3e-9d4f-0a1b2c3d4e5f"),
    ).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "Downloads are switched off for your role.",
    })
  })

  it("a refused question to AI Copilot rejects as permission_denied, not unavailable", async () => {
    stubFetch(() => json({ error: "permission_denied: assistant:use" }, 403))
    await expect(clickhouseQueryService.askQuestion("q")).rejects.toMatchObject({
      code: "permission_denied",
      status: 403,
      message: "You don't have access. This needs the assistant:use permission.",
    })
  })
})
