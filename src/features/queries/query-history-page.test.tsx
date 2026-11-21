// The history page's own behaviour, against the real component: its
// filters reach the client call, the page numbers walk `page`, a filter
// change returns to page 1, a row's SQL builds the studio link, a refused
// row shows what the server told it, a refused page shows the permission
// state, and the empty state says what the window is. The service is spied; nothing leaves the
// test. `next/navigation` is stubbed before any import resolves
// (`mock.module` is hoisted), for the tabs' pathname.
let pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/query-studio/history",
  useRouter: () => ({
    push: (href: string) => {
      pushed.push(href)
    },
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { queryService } from "@/services"
import type { QueryHistoryItem, QueryHistoryPage as HistoryPage } from "@/services/contracts/queries"
import { ServiceError } from "@/services/errors"
import { QueryHistoryPage } from "./query-history-page"

afterEach(() => {
  cleanup()
  mock.restore()
  pushed = []
  visit("/query-studio/history")
})

/** Points the browser at a history address, the way a link does. */
function visit(path: string) {
  const url = new URL(path, "http://localhost:3000").href
  ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(url)
}

function row(over: Partial<QueryHistoryItem> = {}): QueryHistoryItem {
  return {
    id: "q-1",
    sql: "SELECT 1",
    user: "Rina Wijaya",
    at: "2026-10-25T03:00:00.000Z",
    status: "completed",
    durationMs: 42,
    scannedBytes: 1024,
    costUnits: 0.5,
    workloadClass: "hot-analytics",
    engine: "clickhouse",
    cacheAssisted: false,
    tablesRead: [],
    ...over,
  }
}

/** The envelope the server answers with. */
function pageOf(runs: QueryHistoryItem[], total: number, page = 1): HistoryPage {
  return { runs, total, page, pageSize: 50 }
}

/**
 * A page of `count` rows. Each row's SQL names its number: the visible
 * marker a test can look for (a row's id is a React key, never rendered
 * text).
 */
function rowsFrom(first: number, count = 50): QueryHistoryItem[] {
  return Array.from({ length: count }, (_, i) =>
    row({ id: `q-${String(first + i).padStart(3, "0")}`, sql: `SELECT ${first + i}` }),
  )
}

/** A server of 120 runs: page N holds rows 50(N-1)+1 onward, as the real one pages. */
function server120() {
  return spyOn(queryService, "listHistoryPage").mockImplementation((async (filter?: {
    page?: number
  }) => {
    const page = filter?.page ?? 1
    return pageOf(page > 3 ? [] : rowsFrom(50 * (page - 1) + 1, page === 3 ? 20 : 50), 120, page)
  }) as unknown as typeof queryService.listHistoryPage)
}

describe("QueryHistoryPage", () => {
  it("says the window in the empty state, filtered or not", async () => {
    spyOn(queryService, "listHistoryPage").mockResolvedValue(pageOf([], 0))
    render(<QueryHistoryPage />)
    const title = await screen.findByText("No runs in the last 14 days")
    expect(title).toBeDefined()
  })

  it("renders the runs, the refused row's message, and the studio link with the engine", async () => {
    spyOn(queryService, "listHistoryPage").mockResolvedValue(
      pageOf(
        [
          row({
            id: "q-ok",
            sql: "SELECT 1",
            tablesRead: ["marts.retails"],
            durationMs: 42,
          }),
          row({
            id: "q-no",
            sql: "SELECT * FROM bronze.orders",
            status: "blocked",
            message: "Table not found or not available to your organisation: bronze.orders",
            tablesRead: ["bronze.orders"],
          }),
        ],
        2,
      ),
    )
    render(<QueryHistoryPage />)

    const refused = await screen.findByText(
      "Table not found or not available to your organisation: bronze.orders",
    )
    expect(refused).toBeDefined()
    expect(screen.getByText("marts.retails")).toBeDefined()
    // The badge carries the History wording, in the refused run's own row.
    const refusedRow = refused.closest("tr")
    expect(refusedRow).not.toBeNull()
    expect(within(refusedRow!).getByText("Refused")).toBeDefined()
    expect(within(refusedRow!).queryByText("Blocked")).toBeNull()

    const link = screen.getByRole("link", { name: "SELECT * FROM bronze.orders" })
    // QS-12 (LH-QS-4-07), D15: the id travels, not the person's SQL.
    expect(link.getAttribute("href")).toBe("/query-studio?run=q-no")
    expect(link.getAttribute("href")).not.toContain("sql=")
  })

  it("shows Stopped on a stopped run's row", async () => {
    spyOn(queryService, "listHistoryPage").mockResolvedValue(
      pageOf([row({ id: "q-st", sql: "SELECT sleep(9)", status: "stopped", message: "Stopped" })], 1),
    )
    render(<QueryHistoryPage />)
    const link = await screen.findByRole("link", { name: "SELECT sleep(9)" })
    expect(within(link.closest("tr")!).getAllByText("Stopped").length).toBeGreaterThan(0)
    expect(within(link.closest("tr")!).queryByText("Cancelled")).toBeNull()
  })

  it("reads Completed, Failed, Stopped and Refused on the status pills and sends the server's words", async () => {
    const spy = spyOn(queryService, "listHistoryPage").mockResolvedValue(pageOf([], 0))
    render(<QueryHistoryPage />)
    await screen.findByText("No runs in the last 14 days")

    const group = screen.getByRole("group", { name: "Filter by status" })
    const labels = within(group)
      .getAllByRole("button")
      .map((b) => b.textContent)
    expect(labels).toEqual(["All statuses", "Completed", "Failed", "Stopped", "Refused"])

    fireEvent.click(within(group).getByRole("button", { name: "Stopped" }))
    await waitFor(() => expect(spy.mock.calls.at(-1)![0]?.status).toBe("stopped"))
    fireEvent.click(within(group).getByRole("button", { name: "Refused" }))
    await waitFor(() => expect(spy.mock.calls.at(-1)![0]?.status).toBe("blocked"))
  })

  it("sends the filters to the client call", async () => {
    const spy = spyOn(queryService, "listHistoryPage").mockResolvedValue(pageOf([], 0))
    render(<QueryHistoryPage />)
    await screen.findByText("No runs in the last 14 days")

    fireEvent.click(screen.getByRole("button", { name: "Failed" }))
    fireEvent.click(screen.getByRole("button", { name: "Trino" }))
    fireEvent.change(screen.getByLabelText("Minimum duration in milliseconds"), {
      target: { value: "1500" },
    })
    fireEvent.change(screen.getByLabelText("Table contains"), {
      target: { value: "orders" },
    })
    fireEvent.change(screen.getByLabelText("From date"), {
      target: { value: "2026-10-11" },
    })
    fireEvent.change(screen.getByLabelText("To date"), {
      target: { value: "2026-10-20" },
    })

    await waitFor(() => {
      const last = spy.mock.calls.at(-1)!
      expect(last[0]).toEqual({
        status: "failed",
        engine: "trino",
        minDurationMs: 1500,
        table: "orders",
        from: "2026-10-11T00:00:00Z",
        to: "2026-10-20T23:59:59.999Z",
        page: 1,
      })
    })
  })

  it("says Page 1 of 3 for 120 runs and walks the page number with the page buttons", async () => {
    const spy = server120()
    render(<QueryHistoryPage />)

    await screen.findByText("SELECT 50")
    expect(screen.getByText("Page 1 of 3")).toBeDefined()
    expect(screen.queryByText("SELECT 51")).toBeNull()
    expect((screen.getByRole("button", { name: "Previous" }) as HTMLButtonElement).disabled).toBe(true)

    // A page number goes straight to that page; the rows replace, not append.
    fireEvent.click(screen.getByRole("button", { name: "2" }))
    await screen.findByText("SELECT 51")
    expect(spy.mock.calls.at(-1)![0]?.page).toBe(2)
    expect(screen.getByText("Page 2 of 3")).toBeDefined()
    expect(screen.queryByText("SELECT 50")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Next" }))
    await screen.findByText("SELECT 101")
    expect(spy.mock.calls.at(-1)![0]?.page).toBe(3)
    expect(screen.getByText("Page 3 of 3")).toBeDefined()
    expect((screen.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(true)

    fireEvent.click(screen.getByRole("button", { name: "Previous" }))
    await screen.findByText("SELECT 51")
    expect(spy.mock.calls.at(-1)![0]?.page).toBe(2)
    expect(screen.getByText("Page 2 of 3")).toBeDefined()
  })

  it("has no Load more button", async () => {
    server120()
    render(<QueryHistoryPage />)
    await screen.findByText("SELECT 50")
    expect(screen.queryByRole("button", { name: /load more/i })).toBeNull()
  })

  it("rounds the page count up: 51 runs make 2 pages and 50 make 1", async () => {
    spyOn(queryService, "listHistoryPage").mockResolvedValue(pageOf(rowsFrom(1), 51))
    render(<QueryHistoryPage />)
    expect(await screen.findByText("Page 1 of 2")).toBeDefined()
    cleanup()
    mock.restore()

    spyOn(queryService, "listHistoryPage").mockResolvedValue(pageOf(rowsFrom(1), 50))
    render(<QueryHistoryPage />)
    expect(await screen.findByText("Page 1 of 1")).toBeDefined()
  })

  it("returns to page 1 when a filter changes", async () => {
    const spy = server120()
    render(<QueryHistoryPage />)
    await screen.findByText("SELECT 50")
    fireEvent.click(screen.getByRole("button", { name: "2" }))
    await screen.findByText("SELECT 51")

    fireEvent.click(screen.getByRole("button", { name: "Failed" }))

    await waitFor(() => {
      const last = spy.mock.calls.at(-1)![0]
      expect(last?.status).toBe("failed")
      expect(last?.page).toBe(1)
    })
    await screen.findByText("Page 1 of 3")
    // No call ever asked for page 2 with the new filter.
    expect(
      spy.mock.calls.some(([f]) => f?.status === "failed" && (f?.page ?? 1) > 1),
    ).toBe(false)
  })

  it("steps back to the last page when the page asked for is past the end", async () => {
    // The data shrinks to 100 runs (2 pages) after page 1 was drawn with a
    // "3" button, so the click on 3 asks for a page that is gone.
    let total = 120
    const spy = spyOn(queryService, "listHistoryPage").mockImplementation((async (filter?: {
      page?: number
    }) => {
      const page = filter?.page ?? 1
      const last = Math.ceil(total / 50)
      return page > last
        ? pageOf([], total, page)
        : pageOf(rowsFrom(50 * (page - 1) + 1, Math.min(50, total - 50 * (page - 1))), total, page)
    }) as unknown as typeof queryService.listHistoryPage)
    render(<QueryHistoryPage />)
    await screen.findByText("Page 1 of 3")

    total = 100
    fireEvent.click(screen.getByRole("button", { name: "3" }))

    expect(await screen.findByText("Page 2 of 2")).toBeDefined()
    await screen.findByText("SELECT 51")
    expect(spy.mock.calls.map(([f]) => f?.page)).toEqual([1, 3, 2])
  })

  it("a 403 from the history call shows the permission state", async () => {
    spyOn(queryService, "listHistoryPage").mockImplementation((async (filter?: {
      page?: number
    }) => {
      if ((filter?.page ?? 1) > 1) {
        throw new ServiceError(
          "permission_denied",
          "You do not have access to the query history.",
          403,
        )
      }
      return pageOf(rowsFrom(1), 120)
    }) as unknown as typeof queryService.listHistoryPage)
    render(<QueryHistoryPage />)

    await screen.findByText("SELECT 50")
    fireEvent.click(screen.getByRole("button", { name: "2" }))

    expect(await screen.findByText("You don't have access")).toBeDefined()
  })

  // QS-12 (LH-QS-4-07), D16: the whole row opens the run by id, not just its
  // SQL text, and a click that ended a selection does not navigate.
  it("opens the run by its id when the row's time cell is clicked", async () => {
    spyOn(queryService, "listHistoryPage").mockResolvedValue(
      pageOf([row({ id: "q-42" })], 1),
    )
    render(<QueryHistoryPage />)
    const link = await screen.findByRole("link", { name: "SELECT 1" })
    const timeCell = link.closest("tr")!.querySelector("td")!

    fireEvent.click(timeCell)

    expect(pushed).toEqual(["/query-studio?run=q-42"])
  })

  // QS round review R2: a modified click on the SQL link is the browser's
  // "open in a new tab" and must not also navigate this tab; a plain click on
  // the row still pushes exactly once.
  it("does not navigate this tab on a modified click of the SQL link", async () => {
    spyOn(queryService, "listHistoryPage").mockResolvedValue(
      pageOf([row({ id: "q-42" })], 1),
    )
    render(<QueryHistoryPage />)
    const link = await screen.findByRole("link", { name: "SELECT 1" })

    for (const init of [
      { metaKey: true },
      { ctrlKey: true },
      { shiftKey: true },
      { altKey: true },
      { button: 1 },
    ]) {
      fireEvent.click(link, init)
    }

    expect(pushed).toEqual([])
  })

  it("does not navigate this tab when the click came from the SQL link", async () => {
    spyOn(queryService, "listHistoryPage").mockResolvedValue(
      pageOf([row({ id: "q-42" })], 1),
    )
    render(<QueryHistoryPage />)
    const link = await screen.findByRole("link", { name: "SELECT 1" })

    fireEvent.click(link)

    expect(pushed).toEqual([])
  })

  it("does not navigate when the click ended a text selection", async () => {
    spyOn(queryService, "listHistoryPage").mockResolvedValue(
      pageOf([row({ id: "q-42" })], 1),
    )
    render(<QueryHistoryPage />)
    const link = await screen.findByRole("link", { name: "SELECT 1" })
    const timeCell = link.closest("tr")!.querySelector("td")!
    const selection = spyOn(window, "getSelection").mockReturnValue({
      toString: () => "selected",
    } as never)

    fireEvent.click(timeCell)

    expect(pushed).toEqual([])
    selection.mockRestore()
  })

  // QS-12 (LH-QS-4-06), D17: the address holds the filters on entry and is
  // rewritten with replaceState on change, never one history entry per key.
  it("opens with the filters in the address and sends them to the server", async () => {
    visit("/query-studio/history?engine=trino&table=orders")
    const spy = spyOn(queryService, "listHistoryPage").mockResolvedValue(pageOf([], 0))
    render(<QueryHistoryPage />)

    expect(await screen.findByText("No runs match these filters")).toBeTruthy()
    expect(spy.mock.calls[0]?.[0]).toMatchObject({ engine: "trino", table: "orders" })
    expect(
      (screen.getByRole("button", { name: "Trino" }) as HTMLButtonElement).getAttribute(
        "aria-pressed",
      ),
    ).toBe("true")
    expect((screen.getByLabelText("Table contains") as HTMLInputElement).value).toBe("orders")
  })

  it("writes a filter change back to the address with replaceState", async () => {
    spyOn(queryService, "listHistoryPage").mockResolvedValue(pageOf([], 0))
    const replace = spyOn(window.history, "replaceState")
    render(<QueryHistoryPage />)
    await screen.findByText("No runs in the last 14 days")

    fireEvent.click(screen.getByRole("button", { name: "Failed" }))

    await waitFor(() => {
      const href = replace.mock.calls.at(-1)?.[2] as string
      expect(href).toContain("status=failed")
    })
  })

  // QS-12 (LH-QS-4-06), D18: a filter that emptied the list is the thing to
  // blame, with an action that clears it.
  it("blames the filters, not the window, when a filter emptied the list", async () => {
    spyOn(queryService, "listHistoryPage").mockResolvedValue(pageOf([], 0))
    render(<QueryHistoryPage />)
    await screen.findByText("No runs in the last 14 days")

    fireEvent.click(screen.getByRole("button", { name: "Failed" }))

    expect(await screen.findByText("No runs match these filters")).toBeTruthy()
    expect(screen.getByText("Clear a filter and try again.")).toBeTruthy()
  })
})
