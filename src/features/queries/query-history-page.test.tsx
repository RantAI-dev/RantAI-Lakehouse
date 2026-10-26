// The history page's own behaviour, against the real component: its
// filters reach the client call, "Load more" walks the cursor, a row's
// SQL builds the studio link, a refused row shows what the server told
// it, a refused older page says why there is none, and the empty state
// says what the window is. The service is spied; nothing leaves the
// test. `next/navigation` is stubbed before any import resolves
// (`mock.module` is hoisted), for the tabs' pathname.
mock.module("next/navigation", () => ({
  usePathname: () => "/query-studio/history",
}))

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { queryService } from "@/services"
import type { QueryHistoryItem } from "@/services/contracts/queries"
import { ServiceError } from "@/services/errors"
import { QueryHistoryPage } from "./query-history-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

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

/**
 * A full page, the size the server answers with, ids q-001..q-050. Each
 * row's SQL names its number: the visible marker a test can look for (a
 * row's id is a React key, never rendered text).
 */
function fullPage(): QueryHistoryItem[] {
  return Array.from({ length: 50 }, (_, i) =>
    row({ id: `q-${String(i + 1).padStart(3, "0")}`, sql: `SELECT ${i + 1}` }),
  )
}

describe("QueryHistoryPage", () => {
  it("says the window in the empty state, filtered or not", async () => {
    spyOn(queryService, "listHistory").mockResolvedValue([])
    render(<QueryHistoryPage />)
    const title = await screen.findByText("No runs in the last 14 days")
    expect(title).toBeDefined()
  })

  it("renders the runs, the refused row's message, and the studio link with the engine", async () => {
    spyOn(queryService, "listHistory").mockResolvedValue([
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
    ])
    render(<QueryHistoryPage />)

    const refused = await screen.findByText(
      "Table not found or not available to your organisation: bronze.orders",
    )
    expect(refused).toBeDefined()
    expect(screen.getByText("marts.retails")).toBeDefined()
    // The badge carries the status label, in the refused run's own row.
    const refusedRow = refused.closest("tr")
    expect(refusedRow).not.toBeNull()
    expect(within(refusedRow!).getByText("Blocked")).toBeDefined()

    const link = screen.getByRole("link", { name: "SELECT * FROM bronze.orders" })
    expect(link.getAttribute("href")).toBe(
      `/query-studio?sql=${encodeURIComponent("SELECT * FROM bronze.orders")}&engine=clickhouse`,
    )
  })

  it("sends the filters to the client call", async () => {
    const spy = spyOn(queryService, "listHistory").mockResolvedValue([])
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
      })
    })
  })

  it("appends the older page and walks the cursor on Load more", async () => {
    const spy = spyOn(queryService, "listHistory").mockImplementation(
      (async (filter?: { before?: { at: string; id: string } }) => {
        if (filter?.before) {
          return [row({ id: "q-051", sql: "SELECT 51" }), row({ id: "q-052", sql: "SELECT 52" })]
        }
        return fullPage()
      }) as unknown as typeof queryService.listHistory,
    )
    render(<QueryHistoryPage />)

    await screen.findByText("SELECT 50")
    expect(screen.queryByText("SELECT 51")).toBeNull()

    // The button turns on one commit after the rows: the page marks a
    // full page in an effect after the fetch lands.
    fireEvent.click(await screen.findByRole("button", { name: "Load more" }))

    await screen.findByText("SELECT 52")
    expect(screen.getByText("SELECT 50")).toBeDefined()
    // The cursor is the at and id of the last row the first page ended on.
    const last = spy.mock.calls.at(-1)!
    expect(last[0]?.before).toEqual({ at: "2026-10-25T03:00:00.000Z", id: "q-050" })
  })

  it("hides Load more once a page comes back short", async () => {
    spyOn(queryService, "listHistory").mockResolvedValue([row()])
    render(<QueryHistoryPage />)
    await screen.findByText("SELECT 1")
    expect(screen.queryByRole("button", { name: "Load more" })).toBeNull()
  })

  it("a 403 from the history call shows the permission message", async () => {
    spyOn(queryService, "listHistory").mockImplementation(
      (async (filter?: { before?: { at: string; id: string } }) => {
        if (filter?.before) {
          throw new ServiceError(
            "permission_denied",
            "You do not have access to the query history.",
            403,
          )
        }
        return fullPage()
      }) as unknown as typeof queryService.listHistory,
    )
    render(<QueryHistoryPage />)

    await screen.findByText("SELECT 50")
    fireEvent.click(await screen.findByRole("button", { name: "Load more" }))

    const message = await screen.findByText("You do not have access to the query history.")
    expect(message).toBeDefined()
    // The rows the page has stay: the failure only says there is no
    // older page yet.
    expect(screen.getByText("SELECT 50")).toBeDefined()
  })
})
