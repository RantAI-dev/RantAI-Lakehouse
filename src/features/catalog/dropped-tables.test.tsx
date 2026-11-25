// The Catalog's list of dropped tables (`DATA-55`, QA `LH-DATA-15-06`,
// `TC-1731`): each row says who dropped it, how many rows it held and how long
// it has left, and links to the page that restores it.
//
// UX-8: the list moved from a card above the table into a dialog opened from
// the page's ⋯; the facts of a row are one line, joined by " · ". The page's
// own tests cover the ⋯ item that opens it.

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { DroppedTablesList, useDroppedTables } from "./dropped-tables"

/** The list as the page feeds it: the page's one read, rendered. */
function DroppedTablesHarness({ now }: { now: number }) {
  const state = useDroppedTables()
  return <DroppedTablesList state={state} now={now} />
}

const NOW = Date.parse("2026-11-14T00:00:00Z")

afterEach(() => {
  cleanup()
  mock.restore()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

function stubFetch(answer: () => Response) {
  const calls: string[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    calls.push(String(input))
    return answer()
  }) as unknown as typeof fetch)
  return calls
}

const ENTRY = {
  assetId: "orders",
  table: "bronze.orders",
  droppedBy: "Ada Lovelace",
  rowCount: 1234,
  droppedAt: "2026-11-13T08:00:00Z",
  expiresAt: "2026-11-20T00:00:00Z",
}

describe("DroppedTablesList", () => {
  it("lists each dropped table with who dropped it, its rows and the days it has left, and links to its page", async () => {
    const calls = stubFetch(() => json({ tables: [ENTRY] }))
    render(<DroppedTablesHarness now={NOW} />)

    const link = await screen.findByRole("link", { name: /bronze\.orders/ })
    expect(link.getAttribute("href")).toBe("/data/assets/orders")
    expect(screen.getByText("dropped by Ada Lovelace · 1,234 rows · 6 days left")).toBeTruthy()
    expect(calls).toEqual(["/api/catalog/dropped/tables"])
  })

  it("says the rows were not counted rather than 0", async () => {
    stubFetch(() => json({ tables: [{ ...ENTRY, rowCount: null }] }))
    render(<DroppedTablesHarness now={NOW} />)

    expect(await screen.findByText(/· rows not counted ·/)).toBeTruthy()
    expect(screen.queryByText(/0 rows/)).toBeNull()
  })

  it("says one row in the singular, as the dialog does", async () => {
    stubFetch(() => json({ tables: [{ ...ENTRY, rowCount: 1 }] }))
    render(<DroppedTablesHarness now={NOW} />)

    expect(await screen.findByText(/· 1 row ·/)).toBeTruthy()
    expect(screen.queryByText(/1 rows/)).toBeNull()
  })

  it("says less than a day is left for a row at its end", async () => {
    stubFetch(() => json({ tables: [{ ...ENTRY, expiresAt: "2026-11-14T00:00:00Z" }] }))
    render(<DroppedTablesHarness now={NOW} />)

    expect(await screen.findByText(/· less than a day left$/)).toBeTruthy()
  })

  it("says no table is dropped when the list is empty", async () => {
    const calls = stubFetch(() => json({ tables: [] }))
    render(<DroppedTablesHarness now={NOW} />)

    await waitFor(() => expect(calls.length).toBeGreaterThan(0))
    expect(await screen.findByText("No dropped tables.")).toBeTruthy()
  })

  it("says so when the list could not be read", async () => {
    stubFetch(() => json({ error: "database error" }, 503))
    render(<DroppedTablesHarness now={NOW} />)

    expect(await screen.findByText("The list of dropped tables could not be read.")).toBeTruthy()
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy()
  })
})
