import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { EditorView } from "@codemirror/view"
import * as React from "react"
import * as notify from "@/lib/notify"
import { assetService, lakehouseService, queryService } from "@/services"
import { ServiceError } from "@/services/errors"
import type { LakehouseTableDetail } from "@/services/contracts/lakehouse"
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
  parameters: [],
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

  /** The Iceberg table the studio reads to prove a Bronze table is there. */
  const ICEBERG_TABLE = {
    schema: [],
    partitionSpec: [],
    properties: {},
    snapshots: [],
    stats: {
      fileCount: 1,
      smallFileCount: null,
      smallFileThresholdBytes: null,
      recordCount: 1,
      totalBytes: 1,
      snapshotCount: 0,
      metadataLogCount: 0,
    },
  } as unknown as LakehouseTableDetail

  function mountWithBronze(
    options: { tableMissing?: boolean; tableError?: ServiceError } = {}
  ) {
    spyOn(queryService, "listSaved").mockResolvedValue([])
    spyOn(queryService, "listHistory").mockResolvedValue([])
    spyOn(queryService, "estimate").mockRejectedValue(new ServiceError("unavailable", "no estimate"))
    spyOn(assetService, "listAssets").mockResolvedValue([
      { id: "demo_orders", name: "Demo Orders", namespace: "bronze", layer: "raw" },
    ] as never)
    spyOn(assetService, "getAsset").mockResolvedValue(BRONZE_DETAIL as never)
    // The studio asks the Iceberg table whether it is there, before it offers
    // a Bronze table's Trino name (QS-4 D3).
    const tableAnswer = options.tableMissing
      ? () => Promise.reject(new ServiceError("not_found", "table not found", 404))
      : options.tableError
        ? () => Promise.reject(options.tableError)
        : () => Promise.resolve(ICEBERG_TABLE)
    spyOn(lakehouseService, "getTableDetail").mockImplementation(tableAnswer)
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

  it("gives a Bronze table no Trino name when its Iceberg table is not there", async () => {
    visit("/query-studio?sql=SELECT%201&engine=trino")
    const hook = mountWithBronze({ tableMissing: true })
    await waitFor(() => expect(hook.result.current.tables.length).toBe(1))

    act(() => hook.result.current.loadColumns("demo_orders"))

    await waitFor(() =>
      expect(hook.result.current.proofs["demo_orders"]?.status).toBe("missing")
    )
    expect(hook.result.current.tables[0]).toMatchObject({ reference: "", trinoOnly: true })
  })

  it("holds a proof read that failed as failed, not missing", async () => {
    visit("/query-studio?sql=SELECT%201&engine=trino")
    const hook = mountWithBronze({
      tableError: new ServiceError("unavailable", "warehouse down", 503),
    })
    await waitFor(() => expect(hook.result.current.tables.length).toBe(1))

    act(() => hook.result.current.loadColumns("demo_orders"))

    // A 5xx says nothing about the table: the proof is held as failed so
    // the entry can offer a retry, never as "missing" (#448 review
    // SHOULD-FIX).
    await waitFor(() =>
      expect(hook.result.current.proofs["demo_orders"]?.status).toBe("failed")
    )
    expect(hook.result.current.tables[0]).toMatchObject({ reference: "", trinoOnly: true })
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
    // QA LH-QS-4-10: a failure says "failure", so the page keeps its sentence about the failure.
    expect(runs[2]).toMatchObject({ status: "not-run", reason: "failure" })
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
    // QA LH-QS-4-10: Stop says "stopped", not a failure.
    expect(runs.map((entry) => (entry.status === "not-run" ? entry.reason : null))).toEqual(["stopped", "stopped", "stopped"])
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

describe("useQueryStudio parameters", () => {
  it("gives a name the SQL gains a text definition, and a range name a dateRange one", () => {
    const hook = mountWith("SELECT {{region}} WHERE d > {{day.start}} AND d < {{day.end}}")
    expect(hook.result.current.parameters).toEqual([
      { name: "region", type: "text" },
      { name: "day", type: "dateRange" },
    ])
    expect(hook.result.current.tabs[0]?.parameters).toEqual([
      { name: "region", type: "text" },
      { name: "day", type: "dateRange" },
    ])
  })

  it("makes no field of a placeholder in a comment or a string", () => {
    const hook = mountWith("SELECT '{{a}}' -- {{b}}\n/* {{c}} */")
    expect(hook.result.current.parameters).toEqual([])
  })

  it("keeps the value a field was given, and takes it away when the field is emptied", () => {
    const hook = mountWith("SELECT {{region}}")
    act(() => hook.result.current.setParameterValue("region", "EU"))
    expect(hook.result.current.parameterValues).toEqual({ region: "EU" })
    act(() => hook.result.current.setParameterValue("region", undefined))
    expect(hook.result.current.parameterValues).toEqual({})
    expect(Object.hasOwn(hook.result.current.tabs[0]?.values ?? {}, "region")).toBe(false)
  })

  it("hides the field of a name that left the SQL and brings it back with its definition and value", () => {
    const hook = mountWith("SELECT {{region}}")
    act(() =>
      hook.result.current.setParameterDefinition({
        name: "region",
        type: "dropdown",
        options: [{ value: "eu", label: "Europe" }],
      })
    )
    act(() => hook.result.current.setParameterValue("region", "eu"))

    act(() => hook.result.current.setSql("SELECT 1"))
    expect(hook.result.current.parameters).toEqual([])
    expect(hook.result.current.tabs[0]?.parameters.map((def) => def.name)).toEqual(["region"])
    expect(hook.result.current.parameterValues).toEqual({ region: "eu" })

    act(() => hook.result.current.setSql("SELECT {{region}}"))
    expect(hook.result.current.parameters).toEqual([
      { name: "region", type: "dropdown", options: [{ value: "eu", label: "Europe" }] },
    ])
    expect(hook.result.current.parameterValues).toEqual({ region: "eu" })
  })

  it("clears a value that no longer fits when the type changes", () => {
    const hook = mountWith("SELECT {{region}}")
    act(() => hook.result.current.setParameterValue("region", "EU"))
    act(() => hook.result.current.setParameterDefinition({ name: "region", type: "number" }))
    expect(hook.result.current.parameters).toEqual([{ name: "region", type: "number" }])
    expect(hook.result.current.parameterValues).toEqual({})
  })

  it("writes the definitions and values to the draft", async () => {
    const hook = mountWith("SELECT {{region}}")
    act(() => hook.result.current.setParameterValue("region", "EU"))
    await waitFor(() => {
      const stored = JSON.parse(window.localStorage.getItem("rantai-lakehouse:query-studio-draft") ?? "{}")
      expect(stored.tabs?.[0]?.values).toEqual({ region: "EU" })
      expect(stored.tabs?.[0]?.parameters).toEqual([{ name: "region", type: "text" }])
    })
  })

  it("completes the tab's defined names after {{, including one the SQL does not use yet", () => {
    const hook = mountWith("SELECT {{region}}")
    act(() => hook.result.current.setParameterDefinition({ name: "spare", type: "number" }))
    const sql = "SELECT {{region}}, {{"
    act(() => hook.result.current.setSql(sql))
    const context = {
      state: { doc: { toString: () => sql } },
      pos: sql.length,
    } as unknown as Parameters<typeof hook.result.current.completionSource>[0]
    const answer = hook.result.current.completionSource(context) as unknown as {
      options: { label: string }[]
    }
    expect(answer.options.map((option) => option.label)).toEqual(["region", "spare"])
  })
})

describe("useQueryStudio parameter requests", () => {
  const ESTIMATE = { estimatedBytes: null } as never
  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

  /** Mounts the hook over a statement with the estimate and the lists answered. */
  function mountParams(sql: string) {
    spyOn(queryService, "estimate").mockResolvedValue(ESTIMATE)
    return mountWith(sql)
  }

  /** The options the first `run` call got, and the ones of call `at`. */
  function optionsOf(run: { mock: { calls: unknown[][] } }, at = 0) {
    return run.mock.calls[at]?.[1] as Record<string, unknown>
  }

  it("sends the run the options it sent before parameters when the SQL has no placeholder", async () => {
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    const { result } = mountParams("SELECT 1")
    act(() => result.current.setParameterValue("region", "EU"))

    await act(async () => {
      await result.current.runQuery()
    })

    // A value the tab holds for a name the SQL does not use is not sent.
    expect(Object.keys(optionsOf(run)).sort()).toEqual(["engine", "runId"])
  })

  it("posts a body byte for byte the one it posted before parameters when the SQL has no placeholder", async () => {
    const calls: { url: string; body: unknown }[] = []
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ url: String(input), body: init?.body })
      return new Response(JSON.stringify(RESULT), { status: 200, headers: { "Content-Type": "application/json" } })
    }) as unknown as typeof fetch)
    const { result } = mountParams("SELECT 1")

    await act(async () => {
      await result.current.runQuery()
    })

    const posted = calls.find((call) => call.url.endsWith("/api/query/run"))
    expect(posted?.body).toBe('{"sql":"SELECT 1","engine":"clickhouse"}')
  })

  it("posts the values and the definitions when the SQL has a placeholder", async () => {
    const calls: { url: string; body: unknown }[] = []
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ url: String(input), body: init?.body })
      return new Response(JSON.stringify(RESULT), { status: 200, headers: { "Content-Type": "application/json" } })
    }) as unknown as typeof fetch)
    const { result } = mountParams("SELECT {{region}}")
    act(() => result.current.setParameterValue("region", "EU"))

    await act(async () => {
      await result.current.runQuery()
    })

    const posted = calls.find((call) => call.url.endsWith("/api/query/run"))
    expect(JSON.parse(String(posted?.body))).toStrictEqual({
      sql: "SELECT {{region}}",
      engine: "clickhouse",
      parameters: { region: "EU" },
      definitions: [{ name: "region", type: "text" }],
    })
  })

  it("sends the definitions with their defaults and the values only for names that have one", async () => {
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    const { result } = mountParams("SELECT {{region}}, {{n}}")
    act(() => result.current.setParameterDefinition({ name: "region", type: "text", default: "EU" }))
    act(() => result.current.setParameterValue("n", 3))

    await act(async () => {
      await result.current.runQuery()
    })

    const options = optionsOf(run)
    // The default lives in the definition; it is not copied into the values.
    expect(options.parameters).toStrictEqual({ n: 3 })
    expect(options.definitions).toStrictEqual([
      { name: "region", type: "text", default: "EU" },
      { name: "n", type: "text" },
    ])
  })

  it("sends the definitions of the names the SQL uses, and not the one of a name that left it", async () => {
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    const { result } = mountParams("SELECT {{kept}}, {{left}}")
    act(() =>
      result.current.setParameterDefinition({ name: "left", type: "number", default: 1 })
    )
    act(() => result.current.setParameterValue("left", 2))
    act(() => result.current.setParameterValue("kept", "x"))
    act(() => result.current.setSql("SELECT {{kept}}"))
    // The tab still holds the definition and the value of the name that left.
    expect(result.current.tabs[0]?.parameters.map((def) => def.name)).toEqual(["kept", "left"])
    expect(result.current.tabs[0]?.values).toEqual({ kept: "x", left: 2 })

    await act(async () => {
      await result.current.runQuery()
    })

    const options = optionsOf(run)
    expect(options.parameters).toStrictEqual({ kept: "x" })
    expect(options.definitions).toStrictEqual([{ name: "kept", type: "text" }])
  })

  it("sends the same values and definitions with each statement of a run all", async () => {
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    const { result } = mountParams("SELECT {{a}}; SELECT {{a}} + 1; SELECT 3")
    act(() => result.current.setParameterValue("a", "x"))

    await act(async () => {
      await result.current.runQuery()
    })

    expect(run).toHaveBeenCalledTimes(3)
    for (const at of [0, 1, 2]) {
      expect(optionsOf(run, at).parameters).toStrictEqual({ a: "x" })
      expect(optionsOf(run, at).definitions).toStrictEqual([{ name: "a", type: "text" }])
    }
  })

  it("shows the server's sentence and marks the field it names, and only that one", async () => {
    spyOn(queryService, "run").mockRejectedValue(
      new ServiceError("invalid_request", "Fill in {{region}} to run this query.")
    )
    const { result } = mountParams("SELECT {{region}}, {{ab}}")

    await act(async () => {
      await result.current.runQuery()
    })

    const open = result.current.activeRun
    expect(open?.status === "error" && open.error.message).toBe("Fill in {{region}} to run this query.")
    expect([...result.current.invalidParameters]).toEqual(["region"])
  })

  it("marks no field for a refusal that names none", async () => {
    spyOn(queryService, "run").mockRejectedValue(
      new ServiceError("invalid_request", "This query has a {{ that is not a parameter.")
    )
    const { result } = mountParams("SELECT {{region}}")

    await act(async () => {
      await result.current.runQuery()
    })

    expect(result.current.invalidParameters.size).toBe(0)
  })

  it("marks no field for a failure that is not a refusal of the request", async () => {
    spyOn(queryService, "run").mockRejectedValue(
      new ServiceError("unavailable", "Fill in {{region}} to run this query.")
    )
    const { result } = mountParams("SELECT {{region}}")

    await act(async () => {
      await result.current.runQuery()
    })

    expect(result.current.invalidParameters.size).toBe(0)
  })

  it("takes the mark off a field when its value or its definition changes, and off all when the next run starts", async () => {
    const run = spyOn(queryService, "run").mockRejectedValue(
      new ServiceError("invalid_request", "{{a}} and {{b}} are both wrong")
    )
    const { result } = mountParams("SELECT {{a}}, {{b}}")
    await act(async () => {
      await result.current.runQuery()
    })
    expect([...result.current.invalidParameters].sort()).toEqual(["a", "b"])

    act(() => result.current.setParameterValue("a", "x"))
    expect([...result.current.invalidParameters]).toEqual(["b"])

    act(() => result.current.setParameterDefinition({ name: "b", type: "number" }))
    expect(result.current.invalidParameters.size).toBe(0)

    // The next run starts clean, then marks again what the new refusal names.
    run.mockRejectedValue(new ServiceError("invalid_request", "Fill in {{a}} to run this query."))
    await act(async () => {
      await result.current.runQuery()
    })
    expect([...result.current.invalidParameters]).toEqual(["a"])
    run.mockImplementation(() => new Promise(() => {}))
    await act(async () => {
      void result.current.runQuery()
    })
    expect(result.current.invalidParameters.size).toBe(0)
  })

  it("marks the field in no other tab", async () => {
    spyOn(queryService, "run").mockRejectedValue(
      new ServiceError("invalid_request", "Fill in {{region}} to run this query.")
    )
    const { result } = mountParams("SELECT {{region}}")
    await act(async () => {
      await result.current.runQuery()
    })
    expect(result.current.invalidParameters.size).toBe(1)

    act(() => {
      result.current.addTab({ sql: "SELECT {{region}}" })
    })

    expect(result.current.invalidParameters.size).toBe(0)
  })

  it("skips the estimate while a name has no value and no default, and asks once it has one", async () => {
    const estimate = spyOn(queryService, "estimate").mockResolvedValue(ESTIMATE)
    const { result } = mountWith("SELECT {{region}}")

    await sleep(700)
    expect(estimate).not.toHaveBeenCalled()
    // The panel says what it says for an empty editor, and shows no error.
    expect(result.current.estimatable).toBe(false)
    expect(result.current.estimateAct.status).toBe("idle")

    act(() => result.current.setParameterValue("region", "EU"))
    await waitFor(() => expect(estimate).toHaveBeenCalledTimes(1))
    expect(estimate.mock.calls[0]?.[0]).toBe("SELECT {{region}}")
    expect(estimate.mock.calls[0]?.[1]).toStrictEqual({
      parameters: { region: "EU" },
      definitions: [{ name: "region", type: "text" }],
    })
    expect(result.current.estimatable).toBe(true)
  })

  it("does not skip the estimate for a name that has a default", async () => {
    const estimate = spyOn(queryService, "estimate").mockResolvedValue(ESTIMATE)
    const { result } = mountWith("SELECT {{region}}")
    act(() => result.current.setParameterDefinition({ name: "region", type: "text", default: "EU" }))

    await waitFor(() => expect(estimate).toHaveBeenCalledTimes(1))
    expect(estimate.mock.calls[0]?.[1]).toStrictEqual({
      parameters: {},
      definitions: [{ name: "region", type: "text", default: "EU" }],
    })
  })

  it("asks again when a value changes", async () => {
    const estimate = spyOn(queryService, "estimate").mockResolvedValue(ESTIMATE)
    const { result } = mountWith("SELECT {{region}}")
    act(() => result.current.setParameterValue("region", "EU"))
    await waitFor(() => expect(estimate).toHaveBeenCalledTimes(1))

    act(() => result.current.setParameterValue("region", "US"))

    await waitFor(() => expect(estimate).toHaveBeenCalledTimes(2))
    expect(estimate.mock.calls[1]?.[1]).toStrictEqual({
      parameters: { region: "US" },
      definitions: [{ name: "region", type: "text" }],
    })
  })

  it("estimates SQL without a placeholder as it did before parameters", async () => {
    const estimate = spyOn(queryService, "estimate").mockResolvedValue(ESTIMATE)
    mountWith("SELECT 1")

    await waitFor(() => expect(estimate).toHaveBeenCalledTimes(1))
    expect(estimate.mock.calls[0]?.[0]).toBe("SELECT 1")
    expect(estimate.mock.calls[0]?.[1]).toBeUndefined()
  })

  it("marks the field an estimate's refusal names", async () => {
    spyOn(queryService, "estimate").mockRejectedValue(
      new ServiceError("invalid_request", "{{region}} is inside quotes: write it without them, for example = {{region}}.")
    )
    const { result } = mountWith("SELECT {{region}}")
    act(() => result.current.setParameterValue("region", "EU"))

    await waitFor(() => expect([...result.current.invalidParameters]).toEqual(["region"]))
  })

  it("hands the save sheet no definitions while the tab holds several statements and none is picked", () => {
    const { result } = mountParams("SELECT {{a}}; SELECT {{b}}")
    act(() => result.current.setParameterDefinition({ name: "a", type: "text", default: "EU" }))
    act(() => result.current.setParameterDefinition({ name: "b", type: "number" }))
    act(() => result.current.setParameterValue("a", "US"))
    act(() => result.current.setParameterValue("b", 7))
    expect(result.current.targetStatement).toBeNull()

    expect(result.current.saveParameters).toEqual([])
  })

  it("saves the definitions of the one statement the tab holds, with the default and not the value", () => {
    const { result } = mountParams("SELECT {{a}}")
    act(() => result.current.setParameterDefinition({ name: "a", type: "text", default: "EU" }))
    act(() => result.current.setParameterValue("a", "US"))

    expect(result.current.saveParameters).toStrictEqual([{ name: "a", type: "text", default: "EU" }])
  })

  it("saves nothing for a statement without a placeholder, whatever the tab still holds", () => {
    const { result } = mountParams("SELECT {{a}}")
    act(() => result.current.setParameterDefinition({ name: "a", type: "text", default: "EU" }))
    act(() => result.current.setSql("SELECT 1"))

    expect(result.current.saveParameters).toEqual([])
  })

  it("sends the definitions a save-back is given to the saved query it changes", async () => {
    const { hook } = await mountWithSaved("edit")
    const updateSaved = spyOn(queryService, "updateSaved").mockResolvedValue(SAVED)
    const definitions = [{ name: "a", type: "text" as const, default: "EU" }]

    await act(async () => {
      await hook.result.current.save({
        title: "Orders",
        sql: "SELECT {{a}}",
        tags: [],
        parameters: definitions,
      })
    })

    expect(updateSaved.mock.calls[0]?.[1]).toStrictEqual({
      title: "Orders",
      sql: "SELECT {{a}}",
      tags: [],
      parameters: definitions,
    })
  })

  it("lets a save-back clear the parameters with an empty list, and leaves them alone without the key", async () => {
    const { hook } = await mountWithSaved("edit")
    const updateSaved = spyOn(queryService, "updateSaved").mockResolvedValue(SAVED)

    await act(async () => {
      await hook.result.current.save({ title: "Orders", sql: "SELECT 1", tags: [], parameters: [] })
      await hook.result.current.save({ title: "Orders", sql: "SELECT 1", tags: [] })
    })

    expect(updateSaved.mock.calls[0]?.[1]).toStrictEqual({
      title: "Orders",
      sql: "SELECT 1",
      tags: [],
      parameters: [],
    })
    expect(updateSaved.mock.calls[1]?.[1]).toStrictEqual({ title: "Orders", sql: "SELECT 1", tags: [] })
  })

  it("opens a saved query with its definitions and no values, so each field starts at its default", async () => {
    window.localStorage.clear()
    spyOn(queryService, "listSaved").mockResolvedValue([])
    spyOn(queryService, "listHistory").mockResolvedValue([])
    spyOn(queryService, "estimate").mockResolvedValue(ESTIMATE)
    spyOn(assetService, "listAssets").mockResolvedValue([])
    const definitions = [
      { name: "region", type: "text" as const, default: "EU" },
      {
        name: "tier",
        type: "dropdown" as const,
        default: "gold",
        options: [{ value: "gold", label: "Gold" }],
      },
    ]
    spyOn(queryService, "getSaved").mockResolvedValue({
      ...SAVED,
      sql: "SELECT {{region}}, {{tier}}",
      parameters: definitions,
    })
    visit("/query-studio?saved=q1")
    const hook = renderHook(() => useQueryStudio())
    await waitFor(() => expect(hook.result.current.tabs[0]?.savedId).toBe("q1"))

    expect(hook.result.current.tabs[0]?.parameters).toEqual(definitions)
    expect(hook.result.current.tabs[0]?.values).toEqual({})
    expect(hook.result.current.parameters).toEqual(definitions)

    // A run from that tab gives no value and sends the definitions, defaults in them.
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    await act(async () => {
      await hook.result.current.runQuery()
    })
    expect(optionsOf(run).parameters).toStrictEqual({})
    expect(optionsOf(run).definitions).toStrictEqual(definitions)
  })

  it("puts a saved query's definitions in the open tab when its SQL is loaded from the list", () => {
    const { result } = mountParams("SELECT {{old}}")
    act(() => result.current.setParameterValue("old", "x"))
    const definitions = [{ name: "region", type: "text" as const, default: "EU" }]

    act(() => result.current.loadSql("SELECT {{region}}", definitions))

    expect(result.current.tabs[0]?.parameters).toEqual(definitions)
    expect(result.current.tabs[0]?.values).toEqual({})
  })

  it("keeps the tab's definitions when a statement without any is loaded", () => {
    const { result } = mountParams("SELECT {{old}}")
    act(() => result.current.setParameterValue("old", "x"))

    act(() => result.current.loadSql("SELECT {{old}} + 1"))

    expect(result.current.tabs[0]?.values).toEqual({ old: "x" })
  })
})
