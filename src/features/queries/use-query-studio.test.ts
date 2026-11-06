import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { EditorView } from "@codemirror/view"
import * as React from "react"
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
  // The draft the studio keeps is in this browser: a test that leaves one
  // behind would hand it to the next test, which would then open a tab it
  // never asked for.
  window.localStorage.clear()
  visit("/query-studio")
})

const FAILED = "SELECT amont FROM t"
const PROPOSED = "SELECT amount FROM t"
const REASON = "The column is named amount."

/** Points the browser at a Query Studio address, the way an href does. */
function visit(path: string) {
  const url = new URL(path, "http://localhost:3000").href
  ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(url)
}

/** One saved query, as the server sends it. */
const SAVED = {
  id: "q1",
  title: "Orders",
  sql: "SELECT 1",
  owner: "Rina Wijaya",
  updatedAt: "2026-11-04T10:00:00.000Z",
  tags: ["finance"],
  folderId: null,
  ownerId: "u1",
  level: "edit",
}

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

/**
 * Mounts the hook on a `?saved=` link and waits for the tab it opens.
 *
 * The saved query carries the level the caller holds on it: the level says
 * whether "Save" may change that query or must make a copy of it.
 */
async function mountWithSaved(level: string) {
  window.localStorage.clear()
  spyOn(queryService, "listSaved").mockResolvedValue([])
  spyOn(queryService, "listHistory").mockResolvedValue([])
  spyOn(queryService, "estimate").mockRejectedValue(new ServiceError("unavailable", "no estimate"))
  spyOn(assetService, "listAssets").mockResolvedValue([])
  spyOn(queryService, "getSaved").mockResolvedValue({ ...SAVED, level })
  visit("/query-studio?saved=q1")
  const hook = renderHook(() => useQueryStudio())
  await waitFor(() => expect(hook.result.current.tabs[0]?.savedId).toBe("q1"))
  return { hook }
}

describe("useQueryStudio saved-query link", () => {
  it("opens the saved query the link names, with its id and the caller's level", async () => {
    window.localStorage.clear()
    spyOn(queryService, "listSaved").mockResolvedValue([])
    spyOn(queryService, "listHistory").mockResolvedValue([])
    spyOn(queryService, "estimate").mockRejectedValue(new ServiceError("unavailable", "no estimate"))
    spyOn(assetService, "listAssets").mockResolvedValue([])
    const getSaved = spyOn(queryService, "getSaved").mockResolvedValue(SAVED)
    visit("/query-studio?saved=q1")

    const hook = renderHook(() => useQueryStudio())

    await waitFor(() => expect(getSaved).toHaveBeenCalledTimes(1))
    expect(getSaved.mock.calls[0]?.[0]).toBe("q1")
    await waitFor(() => expect(hook.result.current.tabs[0]?.savedId).toBe("q1"))
    expect(hook.result.current.tabs.length).toBe(1)
    expect(hook.result.current.tabs[0]?.name).toBe("Orders")
    expect(hook.result.current.tabs[0]?.sql).toBe("SELECT 1")
    expect(hook.result.current.tabs[0]?.level).toBe("edit")
  })

  it("says the query is not available and opens no tab when the caller may not see it", async () => {
    window.localStorage.clear()
    spyOn(queryService, "listSaved").mockResolvedValue([])
    spyOn(queryService, "listHistory").mockResolvedValue([])
    spyOn(queryService, "estimate").mockRejectedValue(new ServiceError("unavailable", "no estimate"))
    spyOn(assetService, "listAssets").mockResolvedValue([])
    spyOn(queryService, "getSaved").mockRejectedValue(
      new ServiceError("not_found", "That saved query could not be found.")
    )
    visit("/query-studio?saved=q1")

    const hook = renderHook(() => useQueryStudio())

    await waitFor(() => expect(hook.result.current.savedLinkError).not.toBeNull())
    expect(hook.result.current.savedLinkError).toBe("This query is not available to you.")
    // No tab was opened for it: the tab bar still holds the empty one.
    expect(hook.result.current.tabs.length).toBe(1)
    expect(hook.result.current.tabs[0]?.savedId).toBeUndefined()
  })

  it("does not ask for a saved query when the link names none", async () => {
    window.localStorage.clear()
    spyOn(queryService, "listSaved").mockResolvedValue([])
    spyOn(queryService, "listHistory").mockResolvedValue([])
    spyOn(queryService, "estimate").mockRejectedValue(new ServiceError("unavailable", "no estimate"))
    spyOn(assetService, "listAssets").mockResolvedValue([])
    const getSaved = spyOn(queryService, "getSaved").mockResolvedValue(SAVED)

    const hook = renderHook(() => useQueryStudio())

    await waitFor(() => expect(hook.result.current.tabs.length).toBe(1))
    expect(getSaved).not.toHaveBeenCalled()
  })

  it("opens the saved query once when the page runs in StrictMode", async () => {
    window.localStorage.clear()
    spyOn(queryService, "listSaved").mockResolvedValue([])
    spyOn(queryService, "listHistory").mockResolvedValue([])
    spyOn(queryService, "estimate").mockRejectedValue(new ServiceError("unavailable", "no estimate"))
    spyOn(assetService, "listAssets").mockResolvedValue([])
    const getSaved = spyOn(queryService, "getSaved").mockResolvedValue(SAVED)
    visit("/query-studio?saved=q1")

    // StrictMode runs every effect twice: the link is read once, and the
    // query it names opens in one tab, not two.
    const hook = renderHook(() => useQueryStudio(), {
      wrapper: ({ children }) =>
        React.createElement(React.StrictMode, null, children),
    })

    await waitFor(() => expect(hook.result.current.tabs[0]?.savedId).toBe("q1"))
    expect(getSaved).toHaveBeenCalledTimes(1)
    expect(hook.result.current.tabs.length).toBe(1)
  })
})

describe("useQueryStudio saving", () => {
  it("changes the saved query the tab came from, and writes no folder", async () => {
    const { hook } = await mountWithSaved("edit")
    const updateSaved = spyOn(queryService, "updateSaved").mockResolvedValue(SAVED)
    const saveQuery = spyOn(queryService, "saveQuery").mockResolvedValue(SAVED)

    // The tab came from a query the caller may change, so the button offers
    // a change of it rather than a copy of it.
    expect(hook.result.current.saveTarget.label).toBe("Save")
    expect(hook.result.current.saveTarget.mode).toBe("update")

    await act(async () => {
      await hook.result.current.save({
        title: "Orders",
        sql: "SELECT 2",
        tags: ["finance"],
        folderId: "f1",
      })
    })

    expect(updateSaved).toHaveBeenCalledTimes(1)
    expect(updateSaved.mock.calls[0]?.[0]).toBe("q1")
    expect(updateSaved.mock.calls[0]?.[1]).toEqual({
      title: "Orders",
      sql: "SELECT 2",
      tags: ["finance"],
    })
    expect(saveQuery).not.toHaveBeenCalled()
    expect(hook.result.current.tabs[0]?.savedId).toBe("q1")
  })

  it("makes a copy when the caller may only view the query", async () => {
    const { hook } = await mountWithSaved("view")
    const updateSaved = spyOn(queryService, "updateSaved").mockResolvedValue(SAVED)
    const saveQuery = spyOn(queryService, "saveQuery").mockResolvedValue(SAVED)

    // The tab carries a query the caller may only read, so the button offers
    // a copy of it rather than a change of it.
    expect(hook.result.current.saveTarget.label).toBe("Save as new")
    expect(hook.result.current.saveTarget.mode).toBe("create")

    await act(async () => {
      await hook.result.current.save({ title: "Copy", sql: "SELECT 2", tags: [] })
    })

    expect(saveQuery).toHaveBeenCalledTimes(1)
    expect(updateSaved).not.toHaveBeenCalled()
  })

  it("keeps the id and the level the server answered with, so the tab is the query it made", async () => {
    const { hook } = await mountWithSaved("view")
    const saveQuery = spyOn(queryService, "saveQuery").mockResolvedValue({
      ...SAVED,
      id: "q2",
      level: "edit",
    })

    await act(async () => {
      await hook.result.current.save({ title: "Copy", sql: "SELECT 2", tags: [] })
    })

    expect(saveQuery).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(hook.result.current.tabs[0]?.savedId).toBe("q2"))
    expect(hook.result.current.tabs[0]?.level).toBe("edit")
    expect(hook.result.current.saveTarget.label).toBe("Save")
  })
})

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

describe("useQueryStudio Bronze tables", () => {
  /** A Bronze table's detail as the API sends it: ClickHouse `queryTarget`, plus the registry `tableName`. */
  const BRONZE_DETAIL = {
    schema: [{ name: "oid", dataType: "String" }],
    layer: "raw",
    type: "iceberg-table",
    tableName: "orders",
    namespace: "bronze",
    name: "Demo Orders",
    queryTarget: { engine: "clickhouse", table: "icecat_api.`bronze.orders`" },
  }

  function mountWithBronze() {
    spyOn(queryService, "listSaved").mockResolvedValue([])
    spyOn(queryService, "listHistory").mockResolvedValue([])
    spyOn(queryService, "estimate").mockRejectedValue(new ServiceError("unavailable", "no estimate"))
    spyOn(assetService, "listAssets").mockResolvedValue([
      { id: "demo_orders", name: "Demo Orders", namespace: "bronze", layer: "raw" },
    ] as never)
    spyOn(assetService, "getAsset").mockResolvedValue(BRONZE_DETAIL as never)
    return renderHook(() => useQueryStudio())
  }

  it("names the table by its Trino name on a Trino tab, and answers to it by any spelling", async () => {
    visit("/query-studio?sql=SELECT%201&engine=trino")
    const hook = mountWithBronze()
    await waitFor(() => expect(hook.result.current.tables.length).toBe(1))
    expect(hook.result.current.engine).toBe("trino")

    act(() => hook.result.current.loadColumns("demo_orders"))

    await waitFor(() =>
      expect(hook.result.current.tables[0]).toMatchObject({
        reference: "iceberg.bronze.orders",
        trinoOnly: true,
      })
    )
    expect(hook.result.current.tables[0]?.names).toContain("iceberg.bronze.orders")
    expect(hook.result.current.tables[0]?.names).toContain("bronze.orders")
  })

  it("gives the table no name on a ClickHouse tab, whatever the ClickHouse name the detail carries", async () => {
    const hook = mountWithBronze()
    await waitFor(() => expect(hook.result.current.tables.length).toBe(1))
    expect(hook.result.current.engine).toBe("clickhouse")

    act(() => hook.result.current.loadColumns("demo_orders"))
    await waitFor(() => expect(hook.result.current.columns["demo_orders"]?.status).toBe("ready"))

    expect(hook.result.current.tables[0]).toMatchObject({ reference: "", trinoOnly: true })
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

  it("a permission refusal refuses every statement of the batch", async () => {
    const run = spyOn(queryService, "run").mockRejectedValueOnce(
      new ServiceError("permission_denied", "not allowed to run queries")
    )
    const { result } = mountWith("SELECT 1; SELECT 2; SELECT 3")

    await act(async () => {
      await result.current.runQuery()
    })

    expect(run).toHaveBeenCalledTimes(1)
    const runs = result.current.runs[result.current.activeTabId] ?? []
    expect(runs.map((entry) => entry.status)).toEqual(["error", "error", "error"])
    expect(runs[1]?.status === "error" && runs[1].error.code).toBe("permission_denied")
    expect(runs[2]?.status === "error" && runs[2].error.code).toBe("permission_denied")
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
