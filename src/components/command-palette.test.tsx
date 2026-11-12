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
import { AuthProvider } from "@/features/auth/auth-provider"
import { CommandPalette } from "./command-palette"

const originalFetch = global.fetch
const requested: string[] = []
let authPermissions = ["*:*"]

afterEach(() => {
  global.fetch = originalFetch
  requested.length = 0
  pushed.length = 0
  authPermissions = ["*:*"]
  cleanup()
})

function asset(n: number, over: Record<string, unknown> = {}) {
  return { id: `silver.t${n}`, name: `Table ${n}`, namespace: "silver", ...over }
}

function authResponse() {
  return new Response(
    JSON.stringify({
      id: "u1",
      name: "User",
      email: null,
      roles: ["Admin"],
      permissions: authPermissions,
      tenants: [],
    }),
    { status: 200, headers: { "Content-Type": "application/json" } }
  )
}

/** Answer every catalog request with `body` (or a status), and note its URL. */
function stubCatalog(body: unknown, status = 200) {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes("/api/auth/me")) {
      return authResponse()
    }
    requested.push(url)
    return new Response(JSON.stringify(body), {
      status,
      headers: { "Content-Type": "application/json" },
    })
  }) as unknown as typeof fetch
}

async function search(term: string) {
  render(
    <AuthProvider>
      <CommandPalette />
    </AuthProvider>
  )
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

  it("shows an asset's certified or deprecated mark beside its name, and none without one", async () => {
    stubCatalog({
      assets: [
        asset(1, { certification: "certified" }),
        asset(2, { certification: "deprecated" }),
        asset(3),
      ],
    })
    await search("table")
    await screen.findByText("Table 3")
    const row = (name: string) => screen.getByText(name).closest('[cmdk-item=""]') as HTMLElement
    expect(row("Table 1").textContent).toContain("Certified")
    expect(row("Table 2").textContent).toContain("Deprecated")
    expect(row("Table 3").textContent).not.toMatch(/Certified|Deprecated/)
  })

  it("says an approximate match is approximate", async () => {
    stubCatalog({
      assets: [asset(1, { matchedOn: { field: "description", value: "", approximate: true } })],
    })
    await search("revnue")
    expect(await screen.findByText("description · approximate match")).toBeTruthy()
  })

  it("says an approximate name match is approximate, without naming the name (DATA-32)", async () => {
    stubCatalog({
      assets: [asset(1, { matchedOn: { field: "name", value: "", approximate: true } })],
    })
    await search("revnue")
    expect(await screen.findByText("approximate match")).toBeTruthy()
  })

  // QA `LH-DATA-11-01` F1 and `LH-DATA-11-04` (`DATA-54`): the row shows the
  // table's physical name, and the matched description as text.
  it("shows the physical name under the friendly name, and the matched text as text", async () => {
    stubCatalog({
      assets: [
        asset(1, {
          matchedOn: { field: "description", value: "<b>bold</b> revenue", approximate: false },
        }),
      ],
    })
    await search("revenue")
    const row = (await screen.findByText("Table 1")).closest('[cmdk-item=""]') as HTMLElement
    expect(row.textContent).toContain("silver.t1")
    expect(screen.getByText('description: "<b>bold</b> revenue"')).toBeTruthy()
    expect(document.querySelector('[cmdk-item=""] b')).toBeNull()
  })

  it("lists at most eight assets", async () => {
    stubCatalog({ assets: Array.from({ length: 12 }, (_, i) => asset(i + 1)) })
    await search("table")
    await screen.findByText("Table 1")
    expect(screen.queryAllByText(/^Table \d+$/).length).toBe(8)
    expect(screen.queryByText("Table 9")).toBeNull()
  })

  it("asks the server for the eight results it shows (DATA-41 D3)", async () => {
    // The server ranks every match and returns the first `limit` of them:
    // the box must not be served a catalog of ten thousand tables to throw
    // all but eight away.
    stubCatalog({ assets: [asset(1)] })
    await search("table")
    await screen.findByText("Table 1")
    expect(requested[0]).toContain("limit=8")
  })

  // QA `LH-DATA-11-02` (`DATA-54`): the box shows eight tables; when more
  // matched it says how many, so eight does not read as "that is all".
  it("says how many tables matched when it shows only eight of them", async () => {
    stubCatalog({
      assets: Array.from({ length: 8 }, (_, i) => asset(i + 1)),
      total: 23,
    })
    await search("table")
    await screen.findByText("Table 1")
    const text = document.body.textContent ?? ""
    const line = text.indexOf("Showing 8 of 23: refine the search")
    expect(line).toBeGreaterThan(-1)
    expect(line).toBeLessThan(text.indexOf("See all results"))
  })

  it("says nothing about more when every match is shown", async () => {
    stubCatalog({ assets: [asset(1), asset(2)], total: 2 })
    await search("table")
    await screen.findByText("Table 2")
    expect(screen.queryByText(/^Showing /)).toBeNull()
    cleanup()
    requested.length = 0
    // A server that sends no total: all of its answer is on screen.
    stubCatalog({ assets: [asset(1), asset(2)] })
    await search("table")
    await screen.findByText("Table 2")
    expect(screen.queryByText(/^Showing /)).toBeNull()
  })

  it("opens the Catalog with the typed words from See all results", async () => {
    stubCatalog({ assets: [asset(1)] })
    await search("customer revenue")
    fireEvent.click(await screen.findByText("See all results"))
    expect(pushed).toEqual(["/catalog?search=customer%20revenue"])
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

  it("lists See all results after every asset row, whatever is typed", async () => {
    // DATA-11 review SHOULD-FIX 6: `cmdk` reorders rows by its own score of
    // each row's `value`, and the term typed is in the See all row's value.
    stubCatalog({ assets: [asset(1), asset(2), asset(3)] })
    await search("table")
    await screen.findByText("Table 3")
    const rows = Array.from(document.querySelectorAll('[cmdk-item=""]'))
    const at = (text: string) => rows.findIndex((r) => r.textContent?.includes(text))
    const seeAll = at("See all results")
    expect(seeAll).toBeGreaterThan(-1)
    for (const name of ["Table 1", "Table 2", "Table 3"]) {
      expect(at(name)).toBeGreaterThan(-1)
      expect(at(name)).toBeLessThan(seeAll)
    }
  })
})

describe("CommandPalette dashboards and charts", () => {
  /** Answers each URL with its own body. */
  function stubByUrl(answers: Record<string, unknown>) {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes("/api/auth/me")) return authResponse()
      requested.push(url)
      const key = Object.keys(answers).find((k) => url.includes(k))
      return new Response(JSON.stringify(key ? answers[key] : {}), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
    }) as unknown as typeof fetch
  }

  it("does not send catalog search when caller lacks catalog:read (ADM-13 review round 1, S5)", async () => {
    authPermissions = ["dashboard:read"]
    stubCatalog({ assets: [asset(1)] })
    await search("table")
    await new Promise((resolve) => setTimeout(resolve, 300))
    expect(requested.some((u) => u.includes("/api/catalog"))).toBe(false)
  })

  it("does not send dashboard search or shortcuts when caller lacks dashboard:read (ADM-13 review round 1, S5)", async () => {
    authPermissions = ["catalog:read"]
    stubByUrl({
      "/api/dashboard/shortcuts": { favourites: [], recent: [] },
      "/api/dashboard/search": { hits: [] },
    })
    render(
      <AuthProvider>
        <CommandPalette />
      </AuthProvider>
    )
    fireEvent.keyDown(document, { key: "k", ctrlKey: true })
    const input = await screen.findByPlaceholderText(/Search pages/)
    fireEvent.change(input, { target: { value: "orders" } })
    await new Promise((resolve) => setTimeout(resolve, 300))
    expect(requested.some((u) => u.includes("/api/dashboard"))).toBe(false)
  })

  it("finds a dashboard and a chart by name and opens the chart's dashboard", async () => {
    stubByUrl({
      "/api/dashboard/search": {
        hits: [
          { kind: "board", id: "b_1", name: "Hotels overview" },
          { kind: "chart", id: "u_1", name: "Hotels by province", board: "b_2", boardName: "Map demo" },
        ],
      },
      "/api/catalog": { assets: [] },
    })
    await search("hotels")
    expect(await screen.findByText("Hotels overview")).toBeTruthy()
    expect(screen.getByText("on Map demo")).toBeTruthy()
    fireEvent.click(screen.getByText("Hotels by province"))
    expect(pushed).toEqual(["/dashboards/b_2"])
  })

  it("shows the caller's favourites and recent dashboards before anything is typed", async () => {
    stubByUrl({
      "/api/dashboard/shortcuts": {
        favourites: [{ kind: "board", id: "b_1", name: "My favourite" }],
        recent: [{ kind: "board", id: "b_2", name: "Opened lately" }],
      },
    })
    render(
      <AuthProvider>
        <CommandPalette />
      </AuthProvider>
    )
    fireEvent.keyDown(document, { key: "k", ctrlKey: true })
    expect(await screen.findByText("My favourite")).toBeTruthy()
    expect(screen.getByText("Opened lately")).toBeTruthy()
  })

  it("says so when the dashboard search is unavailable, and stays quiet for a caller without access", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes("/api/auth/me")) return authResponse()
      if (url.includes("/api/dashboard/search")) return new Response(JSON.stringify({ error: "boom" }), { status: 503 })
      return new Response(JSON.stringify({ assets: [] }), { status: 200 })
    }) as unknown as typeof fetch
    await search("orders")
    expect(await screen.findByText("Dashboard search is unavailable")).toBeTruthy()
    cleanup()
    requested.length = 0
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes("/api/auth/me")) return authResponse()
      requested.push(url)
      if (url.includes("/api/dashboard/search")) return new Response(JSON.stringify({ error: "forbidden" }), { status: 403 })
      return new Response(JSON.stringify({ assets: [] }), { status: 200 })
    }) as unknown as typeof fetch
    await search("orders")
    await waitFor(() => expect(requested.some((u) => u.includes("/api/dashboard/search"))).toBe(true))
    // Let the refusal settle: a caller without access sees no group and no line.
    await new Promise((resolve) => setTimeout(resolve, 100))
    expect(screen.queryByText("Dashboard search is unavailable")).toBeNull()
  })
})

describe("CommandPalette empty state and recents (BI-18 part C review fix R12)", () => {
  function stubWith(handler: (url: string) => Promise<Response> | Response) {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes("/api/auth/me")) return authResponse()
      return handler(url)
    }) as unknown as typeof fetch
  }
  const ok = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } })

  it("does not say No results above server hits", async () => {
    stubWith((url) =>
      url.includes("/api/dashboard/search")
        ? ok({ hits: [{ kind: "chart", id: "u_1", name: "Choropleth of hotels", board: "b_1", boardName: "Map demo" }] })
        : ok({ assets: [] }),
    )
    await search("choropleth")
    expect(await screen.findByText("Choropleth of hotels")).toBeTruthy()
    expect(screen.queryByText("No results.")).toBeNull()
    expect(screen.queryByText("Searching…")).toBeNull()
  })

  it("says Searching… while the server answers, then No results. when nothing matched anywhere", async () => {
    let release: (r: Response) => void = () => {}
    let asked = false
    stubWith((url) =>
      url.includes("/api/dashboard/search") ? new Promise<Response>((resolve) => { asked = true; release = resolve }) : ok({ assets: [] }),
    )
    await search("zzzzqq")
    expect(await screen.findByText("Searching…")).toBeTruthy()
    expect(screen.queryByText("No results.")).toBeNull()
    await waitFor(() => expect(asked).toBe(true))
    release(ok({ hits: [] }))
    expect(await screen.findByText("No results.")).toBeTruthy()
    expect(screen.queryByText("Searching…")).toBeNull()
  })

  it("leaves a favourite out of Recent dashboards", async () => {
    stubWith((url) =>
      url.includes("/api/dashboard/shortcuts")
        ? ok({
            favourites: [{ kind: "board", id: "b_1", name: "Map demo" }],
            recent: [{ kind: "board", id: "b_1", name: "Map demo" }, { kind: "board", id: "b_2", name: "Other board" }],
          })
        : ok({}),
    )
    render(
      <AuthProvider>
        <CommandPalette />
      </AuthProvider>
    )
    fireEvent.keyDown(document, { key: "k", ctrlKey: true })
    expect(await screen.findByText("Other board")).toBeTruthy()
    expect(screen.getAllByText("Map demo")).toHaveLength(1)
  })
})

describe("CommandPalette catalog order", () => {
  it("keeps the server's order of assets, not cmdk's fuzzy order (QA LH-DATA-11-06)", async () => {
    // "Revenue" would score above "A long rev note" under fuzzy match.
    stubCatalog({
      assets: [
        asset(1, { name: "A long note about rev" }),
        asset(2, { name: "Zzz" }),
        asset(3, { name: "Revenue" }),
      ],
    })
    await search("rev")
    await screen.findByText("Revenue")
    const names = [...document.querySelectorAll('[cmdk-item=""]')]
      .map((el) => el.textContent ?? "")
      .filter((t) => /A long note about rev|Zzz|Revenue/.test(t))
    // A row now ends with its table's physical name (`DATA-54`); the order
    // asserted is the order of the rows.
    expect(names.map((t) => t.replace(/silver\.t\d+$/, ""))).toEqual([
      "A long note about rev",
      "Zzz",
      "Revenue",
    ])
  })
})
