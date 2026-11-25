import { cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { UNSAVED_SOURCE_SENTENCE, useFilterValues } from "./use-filter-values"

// `BI-47` (QA `LH-BI-6-01`): a chart filter's value list reads the chart's own
// relation and names no board; an unsaved SQL source has no list at all; a
// dashboard filter keeps asking by board.

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

type Call = { url: string }

/** Answers every request with one value and records the URLs it was asked. */
function stub(): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL) => {
    calls.push({ url: String(input) })
    return new Response(JSON.stringify({ values: ["Bali"], truncated: false }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    })
  }) as unknown as typeof fetch
  return calls
}

const query = (call: Call) => new URL(call.url, "http://console.invalid").searchParams

describe("useFilterValues", () => {
  it("the value list of a chart filter asks for the chart's own mart and names no board", async () => {
    const calls = stub()
    const { result } = renderHook(() => useFilterValues({ board: "", column: "province", others: [], search: "", source: { mart: "mart_x" } }))
    await waitFor(() => expect(result.current.values).toEqual(["Bali"]))
    expect(calls).toHaveLength(1)
    expect(calls[0].url.startsWith("/api/dashboard/values?")).toBe(true)
    expect(query(calls[0]).get("mart")).toBe("mart_x")
    expect(query(calls[0]).get("column")).toBe("province")
    expect(query(calls[0]).has("board")).toBe(false)
  })

  it("it sends the joins of a joined chart", async () => {
    const calls = stub()
    const joins = [{ mart: "mart_y", type: "left", on: [{ left: "id", op: "=", right: "id" }] }]
    const { result } = renderHook(() => useFilterValues({ board: "", column: "province", others: [], search: "", source: { mart: "mart_x", joins } }))
    await waitFor(() => expect(result.current.values).toEqual(["Bali"]))
    expect(query(calls[0]).get("joins")).toBe(JSON.stringify(joins))
  })

  it("it asks for a SQL source by id", async () => {
    const calls = stub()
    const { result } = renderHook(() => useFilterValues({ board: "", column: "province", others: [], search: "", source: { sqlSource: "s_1" } }))
    await waitFor(() => expect(result.current.values).toEqual(["Bali"]))
    expect(query(calls[0]).get("source")).toBe("s_1")
    expect(query(calls[0]).has("board")).toBe(false)
    expect(query(calls[0]).has("mart")).toBe(false)
  })

  it("an unsaved SQL source makes no request and says to save it", async () => {
    const calls = stub()
    const { result } = renderHook(() => useFilterValues({ board: "", column: "province", others: [], search: "", source: { unsaved: true } }))
    await waitFor(() => expect(result.current.loading).toBe(false))
    expect(calls).toHaveLength(0)
    expect(result.current.values).toEqual([])
    expect(result.current.error).toBe(UNSAVED_SOURCE_SENTENCE)
    expect(UNSAVED_SOURCE_SENTENCE).toBe("Save the SQL source to list this column's values.")
  })

  it("a dashboard filter still asks by board", async () => {
    const calls = stub()
    const { result } = renderHook(() => useFilterValues({ board: "b1", column: "province", others: [], search: "" }))
    await waitFor(() => expect(result.current.values).toEqual(["Bali"]))
    expect(query(calls[0]).get("board")).toBe("b1")
    expect(query(calls[0]).has("mart")).toBe(false)
    expect(query(calls[0]).has("source")).toBe(false)
  })
})
