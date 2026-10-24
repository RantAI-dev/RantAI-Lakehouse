import { afterEach, describe, expect, it, mock } from "bun:test"
import { ServiceError } from "../errors"
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
    expect(err.message).toBe("The assistant could not answer. Try again.")
  })

  it("raises unavailable instead of an empty answer when a 200 response body is not JSON", async () => {
    stubFetch(() => new Response("<html>proxy page</html>", { status: 200 }))
    const err = await rejection(clickhouseQueryService.askQuestion("q"))
    expect(err.code).toBe("unavailable")
    expect(err.message).toBe("The assistant could not answer. Try again.")
  })

  it("raises unavailable instead of an empty answer when a 200 response body is JSON but not an object", async () => {
    stubFetch(() => json(["answer"]))
    const err = await rejection(clickhouseQueryService.askQuestion("q"))
    expect(err.code).toBe("unavailable")
    expect(err.message).toBe("The assistant could not answer. Try again.")
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
    expect(err.message).toBe("This query is too long for the assistant to fix.")
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
