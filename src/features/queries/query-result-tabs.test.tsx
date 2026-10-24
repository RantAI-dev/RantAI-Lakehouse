// The result tabs of a run: one tab per statement, the open one marked,
// and a statement that did not run saying so. It renders the real bar with
// the real hook, because "one tab per statement" is a claim about the
// hook's state. The services are stubbed; nothing here reaches the network.
import { act, cleanup, render, renderHook, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { assetService, queryService } from "@/services"
import { ServiceError } from "@/services/errors"
import { QueryResultTabs } from "./query-result-tabs"
import { useQueryStudio } from "./use-query-studio"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

function stubServices() {
  spyOn(queryService, "listSaved").mockResolvedValue([])
  spyOn(queryService, "listHistory").mockResolvedValue([])
  spyOn(queryService, "estimate").mockRejectedValue(new ServiceError("unavailable", "no estimate"))
  spyOn(assetService, "listAssets").mockResolvedValue([])
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

/** Runs a buffer and draws the bar over what came back. */
async function barOver(sql: string) {
  const hook = renderHook(() => useQueryStudio())
  await act(async () => {
    hook.result.current.setSql(sql)
  })
  await act(async () => {
    await hook.result.current.runQuery()
  })
  return render(<QueryResultTabs studio={hook.result.current} />)
}

/** The buttons of the bar, in the order they are drawn. */
function tabButtons(): HTMLElement[] {
  return within(screen.getByRole("group", { name: "Statement results" })).getAllByRole("button")
}

describe("QueryResultTabs", () => {
  it("shows one tab per statement, in the order the statements were written", async () => {
    stubServices()
    spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    await barOver("SELECT 1; SELECT 2; SELECT 3")

    await waitFor(() => expect(tabButtons().length).toBe(3))
    expect(tabButtons().map((button) => button.textContent)).toEqual([
      "SELECT 1",
      "SELECT 2",
      "SELECT 3",
    ])
  })

  it("marks the tab that is open", async () => {
    stubServices()
    spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    await barOver("SELECT 1; SELECT 2")

    await waitFor(() => expect(tabButtons().length).toBe(2))
    const open = tabButtons().filter((button) => button.getAttribute("aria-current") === "true")
    expect(open.length).toBe(1)
    expect(open[0]?.textContent).toBe("SELECT 1")
  })

  it("says that a statement after the failure did not run", async () => {
    stubServices()
    spyOn(queryService, "run")
      .mockResolvedValueOnce(RESULT as never)
      .mockRejectedValueOnce(new ServiceError("invalid_request", "unknown column"))
    await barOver("SELECT 1; SELECT 2; SELECT 3")

    await waitFor(() => expect(tabButtons().length).toBe(3))
    expect(tabButtons().map((button) => button.textContent)).toEqual([
      "SELECT 1",
      "SELECT 2",
      "Not run",
    ])
    // The tab that says "Not run" still names the statement it is about.
    expect(tabButtons()[2]?.getAttribute("title")).toBe("SELECT 3")
  })
})
