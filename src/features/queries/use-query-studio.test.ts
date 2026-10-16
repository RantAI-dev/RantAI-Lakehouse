import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { assetService, queryService } from "@/services"
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
