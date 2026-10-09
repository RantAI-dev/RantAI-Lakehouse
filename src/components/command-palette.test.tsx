// The ⌘K box (DATA-11 T2): the server decides which assets match a term, by
// description, tag or column as well as name, and the box must show what it
// decided. `cmdk` filters items by their own `value` text, so an asset whose
// match is not in its id or name would vanish without `forceMount`. The
// requests go through the real `assetService` and `apiFetch`; only `fetch` is
// stubbed.
const pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/",
  useSearchParams: () => new URLSearchParams(""),
  useRouter: () => ({
    push: (to: string) => {
      pushed.push(to)
    },
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { CommandPalette } from "./command-palette"

const originalFetch = global.fetch
const requested: string[] = []

afterEach(() => {
  global.fetch = originalFetch
  requested.length = 0
  pushed.length = 0
  cleanup()
})

function asset(n: number, over: Record<string, unknown> = {}) {
  return { id: `silver.t${n}`, name: `Table ${n}`, namespace: "silver", ...over }
}

/** Answer every catalog request with `body` (or a status), and note its URL. */
function stubCatalog(body: unknown, status = 200) {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    requested.push(String(input))
    return new Response(JSON.stringify(body), {
      status,
      headers: { "Content-Type": "application/json" },
    })
  }) as unknown as typeof fetch
}

async function search(term: string) {
  render(<CommandPalette />)
  fireEvent.keyDown(document, { key: "k", ctrlKey: true })
  const input = await screen.findByPlaceholderText(/Search pages/)
  fireEvent.change(input, { target: { value: term } })
}

describe("CommandPalette catalog search", () => {
  it("shows an asset the server matched by a term that is in neither its id nor its name", async () => {
    stubCatalog({
      assets: [
        asset(1, { matchedOn: { field: "column", value: "revenue_amount", approximate: false } }),
      ],
    })
    await search("revenue_amount")
    expect(await screen.findByText("Table 1")).toBeTruthy()
    // The reason line says why.
    expect(screen.getByText("column revenue_amount")).toBeTruthy()
    expect(requested[0]).toContain("/api/catalog?q=revenue_amount")
  })

  it("says an approximate match is approximate", async () => {
    stubCatalog({
      assets: [asset(1, { matchedOn: { field: "description", value: "", approximate: true } })],
    })
    await search("revnue")
    expect(await screen.findByText("description · approximate match")).toBeTruthy()
  })

  it("lists at most eight assets", async () => {
    stubCatalog({ assets: Array.from({ length: 12 }, (_, i) => asset(i + 1)) })
    await search("table")
    await screen.findByText("Table 1")
    expect(screen.queryAllByText(/^Table \d+$/).length).toBe(8)
    expect(screen.queryByText("Table 9")).toBeNull()
  })

  it("opens the Data Explorer with the typed words from See all results", async () => {
    stubCatalog({ assets: [asset(1)] })
    await search("customer revenue")
    fireEvent.click(await screen.findByText("See all results"))
    expect(pushed).toEqual(["/data?search=customer%20revenue"])
  })

  it("says the catalog search is unavailable when the request fails", async () => {
    stubCatalog({ error: "boom" }, 503)
    await search("orders")
    await waitFor(() => expect(screen.getByText("Catalog search is unavailable")).toBeTruthy())
    expect(screen.queryByText("See all results")).toBeNull()
  })

  it("says the caller has no access to the catalog on a 403, not that it is unavailable", async () => {
    stubCatalog({ error: "forbidden" }, 403)
    await search("orders")
    await waitFor(() =>
      expect(screen.getByText("You do not have access to the catalog")).toBeTruthy()
    )
    expect(screen.queryByText("Catalog search is unavailable")).toBeNull()
  })

  it("shows the API's reason when the catalog answers supported: false", async () => {
    stubCatalog({ assets: [], namespaces: [], supported: false, reason: "Set CATALOG_TENANT_ID first." })
    await search("orders")
    await waitFor(() => expect(screen.getByText("Set CATALOG_TENANT_ID first.")).toBeTruthy())
    expect(screen.queryByText("Catalog search is unavailable")).toBeNull()
  })
})
