import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { EditorView } from "@codemirror/view"
import * as notify from "@/lib/notify"
import { assetService, queryService } from "@/services"
import { ServiceError } from "@/services/errors"
import { useQueryStudio } from "./use-query-studio"

// The explain and fix actions of Query Studio: what Accept, Reject and the
// editor do to each other, and that neither action runs the query. The
// services are stubbed; nothing here reaches the network.
afterEach(() => {
  cleanup()
  mock.restore()
})

const FAILED = "SELECT amont FROM t"
const PROPOSED = "SELECT amount FROM t"
const REASON = "The column is named amount."

/** One row of a result, as the API sends it. */
const RESULT = {
  id: "r1",
  columns: ["n"],
  rows: [{ n: 1 }],
  engine: "clickhouse",
  rowCount: 1,
  truncated: false,
  rowLimit: 2000,
  metrics: {
    durationMs: 1,
    scannedBytes: 0,
    costUnits: 0,
    engine: "hot",
    workloadClass: "interactive",
    cacheHit: null,
    pushdowns: null,
    policyObligations: null,
  },
  plan: null,
}

/** Mounts the hook with `sql` in the editor and no network answers for the lists. */
function mountWith(sql: string) {
  spyOn(queryService, "listSaved").mockResolvedValue([])
  spyOn(queryService, "listHistory").mockResolvedValue([])
  spyOn(assetService, "listAssets").mockResolvedValue([])
  const hook = renderHook(() => useQueryStudio())
  act(() => hook.result.current.setSql(sql))
  return hook
}

describe("useQueryStudio fix", () => {
  it("holds the proposal the assistant made, without touching the editor", async () => {
    const { result } = mountWith(FAILED)
    spyOn(queryService, "proposeFix").mockResolvedValue({ sql: PROPOSED, reason: REASON })

    await act(async () => {
      await result.current.fix.run("unknown column")
    })

    await waitFor(() => expect(result.current.fix.result).not.toBeNull())
    expect(result.current.fix.result).toEqual({ sql: PROPOSED, reason: REASON })
    expect(result.current.sql).toBe(FAILED)
  })

  it("puts the proposal in the editor on accept, clears it, and runs nothing", async () => {
    const { result } = mountWith(FAILED)
    spyOn(queryService, "proposeFix").mockResolvedValue({ sql: PROPOSED, reason: REASON })
    const runSpy = spyOn(queryService, "run").mockResolvedValue({} as never)
    await act(async () => {
      await result.current.fix.run("unknown column")
    })

    act(() => result.current.fix.accept())

    expect(result.current.sql).toBe(PROPOSED)
    expect(result.current.fix.result).toBeNull()
    expect(runSpy).not.toHaveBeenCalled()
  })

  it("clears the proposal on reject and leaves the editor as it was", async () => {
    const { result } = mountWith(FAILED)
    spyOn(queryService, "proposeFix").mockResolvedValue({ sql: PROPOSED, reason: REASON })
    const runSpy = spyOn(queryService, "run").mockResolvedValue({} as never)
    await act(async () => {
      await result.current.fix.run("unknown column")
    })

    act(() => result.current.fix.reject())

    expect(result.current.sql).toBe(FAILED)
    expect(result.current.fix.result).toBeNull()
    expect(runSpy).not.toHaveBeenCalled()
  })

  it("clears the proposal when the person edits the SQL, since its diff no longer matches the editor", async () => {
    const { result } = mountWith(FAILED)
    spyOn(queryService, "proposeFix").mockResolvedValue({ sql: PROPOSED, reason: REASON })
    await act(async () => {
      await result.current.fix.run("unknown column")
    })
    expect(result.current.fix.result).not.toBeNull()

    act(() => result.current.setSql(`${FAILED} LIMIT 1`))

    expect(result.current.fix.result).toBeNull()
  })

  it("clears the proposal when the person runs the query again", async () => {
    const { result } = mountWith(FAILED)
    spyOn(queryService, "proposeFix").mockResolvedValue({ sql: PROPOSED, reason: REASON })
    spyOn(queryService, "run").mockRejectedValue(new Error("unknown column"))
    await act(async () => {
      await result.current.fix.run("unknown column")
    })
    expect(result.current.fix.result).not.toBeNull()

    await act(async () => {
      await result.current.runQuery()
    })

    expect(result.current.fix.result).toBeNull()
  })

  it("holds the assistant's text and no proposal when it proposed nothing", async () => {
    const { result } = mountWith(FAILED)
    spyOn(queryService, "proposeFix").mockResolvedValue({ answer: "I need the table's columns." })
    await act(async () => {
      await result.current.fix.run("unknown column")
    })
    expect(result.current.fix.result).toStrictEqual({ answer: "I need the table's columns." })
  })
})

describe("useQueryStudio explain", () => {
  it("holds the explanation for the editor's statement and runs nothing", async () => {
    const { result } = mountWith(FAILED)
    const explainSpy = spyOn(queryService, "explainSql").mockResolvedValue("It selects one column.")
    const runSpy = spyOn(queryService, "run").mockResolvedValue({} as never)

    await act(async () => {
      await result.current.explain.run()
    })

    expect(explainSpy).toHaveBeenCalledWith(FAILED, "clickhouse")
    expect(result.current.explain.result).toBe("It selects one column.")
    expect(runSpy).not.toHaveBeenCalled()
  })
})

describe("useQueryStudio columns", () => {
  it("takes the columns of a table once, however often the statement names it", async () => {
    spyOn(queryService, "listSaved").mockResolvedValue([])
    spyOn(queryService, "listHistory").mockResolvedValue([])
    spyOn(assetService, "listAssets").mockResolvedValue([
      { id: "silver.orders", name: "Orders", namespace: "silver" },
    ] as never)
    const getAsset = spyOn(assetService, "getAsset").mockResolvedValue({
      schema: [{ name: "amount", dataType: "Int64" }],
    } as never)

    const hook = renderHook(() => useQueryStudio())
    await waitFor(() => expect(hook.result.current.tables.length).toBe(1))
    act(() =>
      hook.result.current.setSql("SELECT * FROM silver.orders a JOIN silver.orders b")
    )

    await waitFor(() => expect(getAsset).toHaveBeenCalledTimes(1))
    expect(getAsset).toHaveBeenCalledWith("silver.orders")
    await waitFor(() =>
      expect(hook.result.current.columns["silver.orders"]).toEqual({
        status: "ready",
        columns: [{ name: "amount", dataType: "Int64" }],
      })
    )
  })
})

describe("useQueryStudio statements", () => {
  it("runs every statement of the tab, one request each, in the order written", async () => {
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    const { result } = mountWith("SELECT 1; SELECT 2; SELECT 3")

    await act(async () => {
      await result.current.runQuery()
    })

    expect(run).toHaveBeenCalledTimes(3)
    expect(run.mock.calls.map((call) => call[0])).toEqual([
      "SELECT 1",
      "SELECT 2",
      "SELECT 3",
    ])
    const runs = result.current.runs[result.current.activeTabId] ?? []
    expect(runs.map((entry) => entry.status)).toEqual(["done", "done", "done"])
  })

  it("stops at the first failure and marks the statements after it as not run", async () => {
    const run = spyOn(queryService, "run")
      .mockResolvedValueOnce(RESULT as never)
      .mockRejectedValueOnce(new ServiceError("invalid_request", "unknown column"))
    const { result } = mountWith("SELECT 1; SELECT 2; SELECT 3")

    await act(async () => {
      await result.current.runQuery()
    })

    expect(run).toHaveBeenCalledTimes(2)
    const runs = result.current.runs[result.current.activeTabId] ?? []
    expect(runs.map((entry) => entry.status)).toEqual(["done", "error", "not-run"])
  })

  it("refuses a batch of more than twenty statements before anything runs", async () => {
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    const notifyInfo = spyOn(notify, "notifyInfo")
    const many = Array.from({ length: 21 }, (_, index) => `SELECT ${index}`).join("; ")
    const { result } = mountWith(many)

    await act(async () => {
      await result.current.runQuery()
    })

    expect(run).not.toHaveBeenCalled()
    expect(notifyInfo).toHaveBeenCalledTimes(1)
    expect(String(notifyInfo.mock.calls[0]?.[0] ?? "")).toContain("20")
  })

  it("marks the statement in flight and the ones after it as not run when the batch is stopped", async () => {
    const run = spyOn(queryService, "run").mockImplementation(
      () => new Promise(() => {})
    )
    const { result } = mountWith("SELECT 1; SELECT 2; SELECT 3")

    await act(async () => {
      void result.current.runQuery()
    })
    await waitFor(() => expect(run).toHaveBeenCalledTimes(1))

    await act(async () => {
      result.current.stopRun()
    })

    const runs = result.current.runs[result.current.activeTabId] ?? []
    expect(runs.map((entry) => entry.status)).toEqual(["not-run", "not-run", "not-run"])
    expect(result.current.running).toBe(false)
  })

  it("runs only the selection when there is one", async () => {
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    const sql = "SELECT 1; SELECT 2; SELECT 3"
    const { result } = mountWith(sql)
    // The editor's view is the only place that knows what is selected, so
    // the test holds a real one and picks the second statement in it.
    const view = new EditorView({ doc: sql, parent: document.body })
    act(() => result.current.setEditorView(view))
    const from = sql.indexOf("SELECT 2")
    view.dispatch({ selection: { anchor: from, head: from + "SELECT 2".length } })

    await act(async () => {
      await result.current.runSelectionOrAll()
    })

    expect(run).toHaveBeenCalledTimes(1)
    expect(run.mock.calls[0]?.[0]).toBe("SELECT 2")
    view.destroy()
  })
})
