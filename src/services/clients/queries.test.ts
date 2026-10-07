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
