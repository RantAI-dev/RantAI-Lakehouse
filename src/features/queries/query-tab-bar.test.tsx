// The query tabs inside the SQL editor: what a person sees when they open,
// rename and close a tab, what the browser refuses to hold, and what is
// still there after a reload. It renders the real bar with the real hook,
// because "every tab keeps its own SQL" is a claim about the hook's state,
// not about a callback. The services are stubbed; nothing here reaches the
// network.
import { act, cleanup, fireEvent, render, renderHook, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import * as notify from "@/lib/notify"
import { QUERY_DRAFT_KEY } from "@/lib/query-draft"
import { assetService, queryService } from "@/services"
import { ServiceError } from "@/services/errors"
import { QueryTabBar } from "./query-tab-bar"
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

/** The bar with the hook behind it, and the SQL of the open tab on screen. */
function Harness() {
  const studio = useQueryStudio()
  return (
    <>
      <QueryTabBar studio={studio} />
      <span data-testid="sql">{studio.sql}</span>
    </>
  )
}

function tabBar(): HTMLElement {
  return screen.getByRole("group", { name: "Query tabs" })
}

/**
 * The buttons that carry the tab names. The close, rename and add buttons
 * name the tab they work on, so they are the ones with an `aria-label`.
 */
function tabButtons(): HTMLElement[] {
  return within(tabBar())
    .getAllByRole("button")
    .filter((button) => !button.getAttribute("aria-label"))
}

/** The name of the tab that is open. */
function openTabName(): string | null {
  const open = tabButtons().find((button) => button.getAttribute("aria-current") === "true")
  return open ? (open.textContent ?? "") : null
}

/** The SQL of the open tab, as the editor holds it. */
function openSql(): string {
  return screen.getByTestId("sql").textContent ?? ""
}

/** Stores the tabs the way the browser keeps them. */
function seedTabs(tabs: { id: string; name: string; sql: string; engine: string }[], activeId: string) {
  window.localStorage.setItem(
    QUERY_DRAFT_KEY,
    JSON.stringify({ tabs, activeId, question: "", tab: "sql" })
  )
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

describe("QueryTabBar", () => {
  it("opens a new tab, gives it a name nobody uses, and makes it the open one", async () => {
    stubServices()
    render(<Harness />)

    expect(tabButtons().length).toBe(1)
    expect(openTabName()).toBe("Query 1")

    fireEvent.click(screen.getByRole("button", { name: "Add query tab" }))

    await waitFor(() => expect(tabButtons().length).toBe(2))
    expect(tabButtons().map((button) => button.textContent)).toEqual(["Query 1", "Query 2"])
    expect(openTabName()).toBe("Query 2")
  })

  it("switches to another tab and shows the SQL that belongs to it", async () => {
    stubServices()
    seedTabs(
      [
        { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" },
        { id: "b", name: "Users", sql: "SELECT 2", engine: "trino" },
      ],
      "a"
    )
    render(<Harness />)

    await waitFor(() => expect(tabButtons().length).toBe(2))
    expect(openTabName()).toBe("Orders")
    expect(openSql()).toBe("SELECT 1")

    fireEvent.click(screen.getByRole("button", { name: "Users" }))

    expect(openTabName()).toBe("Users")
    expect(openSql()).toBe("SELECT 2")
  })

  it("renames a tab and keeps the name it had when the new one is empty", async () => {
    stubServices()
    render(<Harness />)

    fireEvent.click(screen.getByRole("button", { name: "Rename Query 1" }))
    const input = await screen.findByRole("textbox", { name: "Rename Query 1" })
    fireEvent.change(input, { target: { value: "Orders" } })
    fireEvent.keyDown(input, { key: "Enter" })

    await waitFor(() => expect(openTabName()).toBe("Orders"))

    // An empty name is refused: the tab keeps the name it had.
    fireEvent.click(screen.getByRole("button", { name: "Rename Orders" }))
    const second = await screen.findByRole("textbox", { name: "Rename Orders" })
    fireEvent.change(second, { target: { value: "  " } })
    fireEvent.keyDown(second, { key: "Enter" })

    await waitFor(() => expect(openTabName()).toBe("Orders"))
  })

  it("closes a tab and opens the one before it", async () => {
    stubServices()
    seedTabs(
      [
        { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" },
        { id: "b", name: "Users", sql: "SELECT 2", engine: "clickhouse" },
      ],
      "b"
    )
    render(<Harness />)

    await waitFor(() => expect(tabButtons().length).toBe(2))
    fireEvent.click(screen.getByRole("button", { name: "Close Users" }))

    await waitFor(() => expect(tabButtons().length).toBe(1))
    expect(tabButtons()[0].textContent).toBe("Orders")
    expect(openTabName()).toBe("Orders")
  })

  it("leaves one empty tab when the last tab is closed", async () => {
    stubServices()
    seedTabs([{ id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" }], "a")
    render(<Harness />)

    await waitFor(() => expect(openSql()).toBe("SELECT 1"))
    fireEvent.click(screen.getByRole("button", { name: "Close Orders" }))

    await waitFor(() => expect(openSql()).toBe(""))
    expect(tabButtons().length).toBe(1)
    expect(openTabName()).toBe("Orders")
  })

  it("refuses a tab beyond the limit the browser keeps and says so", async () => {
    stubServices()
    const notifyInfo = spyOn(notify, "notifyInfo")
    seedTabs(
      Array.from({ length: 20 }, (_, index) => ({
        id: `t${index}`,
        name: `Query ${index + 1}`,
        sql: `SELECT ${index}`,
        engine: "clickhouse",
      })),
      "t0"
    )
    render(<Harness />)

    await waitFor(() => expect(tabButtons().length).toBe(20))
    fireEvent.click(screen.getByRole("button", { name: "Add query tab" }))

    expect(tabButtons().length).toBe(20)
    expect(notifyInfo).toHaveBeenCalledTimes(1)
    expect(String(notifyInfo.mock.calls[0]?.[0] ?? "")).toContain("20")
  })

  it("shows the draft that came before the tabs as one tab named Query 1", async () => {
    stubServices()
    window.localStorage.setItem(
      QUERY_DRAFT_KEY,
      JSON.stringify({ sql: "SELECT 1", question: "", tab: "sql" })
    )
    render(<Harness />)

    await waitFor(() => expect(openSql()).toBe("SELECT 1"))
    expect(tabButtons().length).toBe(1)
    expect(openTabName()).toBe("Query 1")
  })

  it("finds the tabs and their SQL again after a reload", async () => {
    stubServices()
    seedTabs(
      [
        { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" },
        { id: "b", name: "Users", sql: "SELECT 2", engine: "trino" },
      ],
      "b"
    )
    render(<Harness />)

    await waitFor(() => expect(tabButtons().length).toBe(2))
    expect(tabButtons().map((button) => button.textContent)).toEqual(["Orders", "Users"])
    expect(openTabName()).toBe("Users")
    expect(openSql()).toBe("SELECT 2")

    fireEvent.click(screen.getByRole("button", { name: "Orders" }))

    expect(openSql()).toBe("SELECT 1")
  })
})

describe("useQueryStudio results", () => {
  it("keeps the rows with the tab that ran them", async () => {
    stubServices()
    const run = spyOn(queryService, "run").mockResolvedValue(RESULT as never)
    const hook = renderHook(() => useQueryStudio())

    await act(async () => {
      hook.result.current.setSql("SELECT 1")
    })
    await act(async () => {
      await hook.result.current.runQuery()
    })
    expect(run).toHaveBeenCalledTimes(1)

    const first = hook.result.current.activeTabId
    expect(hook.result.current.results[first]).not.toBeNull()

    // The tab that never ran has no rows of its own.
    await act(async () => {
      hook.result.current.addTab()
    })
    const second = hook.result.current.activeTabId
    expect(second).not.toBe(first)
    expect(hook.result.current.results[second]).toBeUndefined()
  })

  it("leaves no rows on a tab whose run failed", async () => {
    stubServices()
    spyOn(queryService, "run")
      .mockResolvedValueOnce(RESULT as never)
      .mockRejectedValueOnce(new ServiceError("invalid_request", "unknown column"))
    const hook = renderHook(() => useQueryStudio())

    await act(async () => {
      hook.result.current.setSql("SELECT 1")
    })
    await act(async () => {
      await hook.result.current.runQuery()
    })
    const tab = hook.result.current.activeTabId
    expect(hook.result.current.results[tab]).not.toBeNull()

    await act(async () => {
      await hook.result.current.runQuery()
    })

    expect(hook.result.current.results[tab]).toBeNull()
  })
})
