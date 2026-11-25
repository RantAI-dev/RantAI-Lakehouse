// DATA-24: the Owner and Tags filters vanished on a laptop-width window.
// The columns were dropped from the table's column list, and a filter lives
// on a column, so the filters went with them — the one thing a narrow
// window took away was the one thing the narrow window was for.
//
// The columns are hidden now, not removed: they stay in the column list, so
// the toolbar's filter picker still offers them at every width, while the
// table itself leaves the room they would take.
//
// WEB-3 S2-D1: Data Explorer and Catalog became one page at `/catalog`. These
// tests moved here with the asset list (they were the Data Explorer's), and
// the old Catalog page's DATA-23 tests are rewritten below for the namespace
// row that replaced its namespace table and drawer.
//
// The page is rendered whole, with `fetch` stubbed, so what is under test is
// what the user is offered: the columns on screen, the filters in the
// picker, and the addresses the quick filters go to.
//
// UX-8 (mockup A): the namespace chips became one searchable picker, the
// layer chips a segmented switch, the table's Settings menu the page's ⋯,
// and the dropped tables a dialog from that ⋯. The quick filters now
// navigate with `router.push` (recorded in `pushed`) instead of being links.

/** The query string the page reads; a test sets it before rendering. */
let currentQuery = ""
/** Every address the page pushed, in order. */
let pushed: string[] = []

mock.module("next/navigation", () => ({
  usePathname: () => "/catalog",
  useSearchParams: () => new URLSearchParams(currentQuery),
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

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { AuthProvider } from "@/features/auth/auth-provider"
import { activeFilterValues, namespaceAssetsHref } from "@/lib/table-filter-link"
import type { Asset, CatalogNamespace } from "@/services/contracts/assets"
import { CatalogPage } from "./catalog-page"

/** A namespace whose display name differs from its id — the DATA-23 case. */
const NAMESPACE: CatalogNamespace = {
  id: "silver",
  name: "Silver Layer",
  description: "Curated tables.",
  assetCount: 12,
  owner: "Data Platform",
  residency: "id-jakarta",
  sourceEngine: "ClickHouse + Iceberg",
}

const ASSET = {
  id: "silver.orders",
  name: "Orders Silver",
  namespace: "silver",
  type: "table",
  layer: "silver",
  tier: "hot",
  classification: "internal",
  owner: "Data Platform",
  domain: "sales",
  description: "One row per order.",
  format: "MergeTree",
  engine: "hot-store",
  rows: 10,
  sizeBytes: 100,
  columnCount: 1,
  freshnessLagSeconds: 60,
  lastUpdated: "2026-10-01T00:00:00Z",
  health: "healthy",
  residency: "id-jakarta",
  tags: ["finance"],
} as unknown as Asset

const ASSET_LIST = [ASSET]

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })

/** Whether the namespace list answers, or fails as an unreachable API would. */
let namespacesFail = false
/** What the signed-in person may do; `connector:manage` decides Add data. */
let permissions: string[] = ["catalog:read"]
/** The rows the list answers with. */
let assetList: Asset[] = ASSET_LIST
/** The dropped tables the API answers with. */
let droppedTables: unknown[] = []

function stubApi() {
  spyOn(globalThis, "fetch").mockImplementation((async (
    input: RequestInfo | URL
  ) => {
    const target = String(input)
    if (target.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Reader", email: null, roles: [], permissions, tenants: [] })
    }
    if (target.startsWith("/api/catalog/query")) {
      return json({
        totalItems: assetList.length,
        totalPages: 1,
        pageSize: 5,
        page: 1,
        items: assetList,
      })
    }
    if (target.startsWith("/api/catalog/dropped/tables")) {
      return json({ tables: droppedTables })
    }
    if (target.startsWith("/api/catalog")) {
      return namespacesFail
        ? json({ error: "unavailable" }, 503)
        : json({ assets: [], namespaces: [NAMESPACE] })
    }
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
}

/** The width the window is at, in pixels. */
function setWindowWidth(width: number) {
  Object.defineProperty(window, "innerWidth", {
    value: width,
    writable: true,
    configurable: true,
  })
}

function renderPage() {
  return render(
    <AuthProvider>
      <TableProviders>
        <CatalogPage />
      </TableProviders>
    </AuthProvider>
  )
}

/** The `filters` entries of the last address the page pushed, for one column. */
function lastPushedFilter(id: string): string[] {
  const href = pushed.at(-1) ?? ""
  return activeFilterValues(new URL(href, "http://x").searchParams.get("filters") ?? "", id)
}

/** Opens the toolbar's filter picker, which lists the filterable columns. */
async function openFilterPicker() {
  const trigger = await screen.findByRole("button", { name: "Filter" })
  fireEvent.click(trigger)
  return screen.findByPlaceholderText("Filter by...")
}

beforeEach(() => {
  currentQuery = ""
  pushed = []
  permissions = ["catalog:read"]
  assetList = ASSET_LIST
  droppedTables = []
  namespacesFail = false
  setWindowWidth(1280)
  // The table remembers its layout per user, so a leftover one would decide
  // which columns this test sees.
  localStorage.clear()
  stubApi()
})

afterEach(() => {
  cleanup()
  mock.restore()
})

describe("the Catalog's columns at a narrow window", () => {
  it("does not show the Owner and Tags columns at 1280px", async () => {
    renderPage()

    // The width is measured after the first render, so wait for the table.
    await screen.findByRole("columnheader", { name: "Name" })
    expect(screen.queryByRole("columnheader", { name: "Owner" })).toBeNull()
    expect(screen.queryByRole("columnheader", { name: "Tags" })).toBeNull()
  })

  it("offers the Owner and Tags filters at 1280px all the same", async () => {
    renderPage()
    const picker = await openFilterPicker()

    expect(picker).toBeTruthy()
    expect(screen.getByRole("option", { name: "Owner" })).toBeTruthy()
    expect(screen.getByRole("option", { name: "Tags" })).toBeTruthy()
  })

  it("shows the Owner and Tags columns at a wide window", async () => {
    setWindowWidth(1800)
    renderPage()

    await screen.findByRole("columnheader", { name: "Name" })
    expect(screen.queryByRole("columnheader", { name: "Owner" })).toBeTruthy()
    expect(screen.queryByRole("columnheader", { name: "Tags" })).toBeTruthy()
  })

  it("folds the columns silently, without a line naming them (UX-8)", async () => {
    renderPage()

    await screen.findByRole("columnheader", { name: "Name" })
    expect(screen.queryByText(/hidden at this window width/)).toBeNull()
  })

  it("keeps the window's hide out of the layout it saves for the user", async () => {
    renderPage()
    await screen.findByRole("columnheader", { name: "Tier" })

    // Hide a column the user can see, the way the header menu does. The
    // window's own hide must not ride along: it is the page's decision, not
    // the user's, and it would keep the column hidden after the window grows.
    fireEvent.pointerDown(screen.getByRole("button", { name: "Tier" }))
    fireEvent.click(await screen.findByRole("menuitemcheckbox", { name: "Hide" }))

    const saved = JSON.parse(
      localStorage.getItem("app:table-layout:/catalog") ?? "{}"
    ) as { columnVisibility?: Record<string, boolean> }
    expect(saved.columnVisibility?.tier).toBe(false)
    expect(saved.columnVisibility).not.toHaveProperty("owner")
    expect(saved.columnVisibility).not.toHaveProperty("tags")
  })
})

describe("the Catalog's namespace picker", () => {
  /** Opens the picker and picks the namespace shown under `name`. */
  async function pickNamespace(name: string) {
    fireEvent.click(await screen.findByRole("button", { name: "Namespace" }))
    fireEvent.click(await screen.findByRole("option", { name: new RegExp(name) }))
  }

  it("filters the list to the namespace id, not its display name (DATA-23)", async () => {
    renderPage()
    await pickNamespace("Silver Layer")

    const href = pushed.at(-1) ?? ""
    expect(new URL(href, "http://x").pathname).toBe("/catalog")
    expect(lastPushedFilter("namespace")).toEqual([NAMESPACE.id])
    expect(href).toBe(namespaceAssetsHref(NAMESPACE.id))
  })

  it("keeps the search term when a namespace is picked", async () => {
    currentQuery = "search=orders"
    renderPage()
    await pickNamespace("Silver Layer")

    expect(new URL(pushed.at(-1) ?? "", "http://x").searchParams.get("search")).toBe("orders")
  })

  it("describes the namespace the list is filtered to in one line, and the picker leads back to all of them", async () => {
    currentQuery = new URL(namespaceAssetsHref(NAMESPACE.id), "http://x").search.slice(1)
    renderPage()

    expect(await screen.findByText("Curated tables.")).toBeTruthy()
    expect(screen.getByText("Owned by Data Platform")).toBeTruthy()
    expect(screen.getByText("id-jakarta")).toBeTruthy()
    expect(screen.getByText("ClickHouse + Iceberg")).toBeTruthy()
    // The picker names the namespace it holds, and offers the way back.
    fireEvent.click(await screen.findByRole("button", { name: "Silver Layer" }))
    fireEvent.click(await screen.findByRole("option", { name: /All namespaces/ }))
    expect(pushed.at(-1)).toBe("/catalog")
  })

  it("says the namespaces could not load, and still lists the assets", async () => {
    namespacesFail = true
    renderPage()

    fireEvent.click(await screen.findByRole("button", { name: "Namespace" }))
    expect(await screen.findByText("Could not load namespaces.")).toBeTruthy()
    const row = screen.getByText("Could not load namespaces.").parentElement
    expect(row ? within(row).getByRole("button", { name: "Retry" }) : null).toBeTruthy()
    expect(await screen.findByText("Orders Silver")).toBeTruthy()
  })
})

describe("the Catalog's layer switch", () => {
  it("filters Bronze to both raw and curated Bronze, and keeps the other filters", async () => {
    currentQuery = new URL(namespaceAssetsHref(NAMESPACE.id), "http://x").search.slice(1)
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Bronze" }))

    expect(lastPushedFilter("layer")).toEqual(["raw", "bronze"])
    expect(lastPushedFilter("namespace")).toEqual([NAMESPACE.id])
  })

  it("presses Bronze for raw and curated Bronze together, and draws no second chip for it", async () => {
    currentQuery = `filters=${encodeURIComponent(
      JSON.stringify([
        { id: "layer", value: ["raw", "bronze"], variant: "multiSelect", operator: "inArray", filterId: "a" },
        { id: "namespace", value: "silver", variant: "text", operator: "eq", filterId: "b" },
      ])
    )}`
    renderPage()
    expect((await screen.findByRole("button", { name: "Bronze" })).getAttribute("aria-pressed")).toBe("true")
    expect(await screen.findByRole("button", { name: "Silver Layer" })).toBeTruthy()
    await screen.findByText("Orders Silver")
    expect(screen.queryByRole("button", { name: "Remove Layer filter" })).toBeNull()
    expect(screen.queryByRole("button", { name: "Remove Namespace filter" })).toBeNull()
  })

  it("leaves a filter the switch cannot show to its chip: raw Bronze alone, or Silver and Gold", async () => {
    for (const value of [["raw"], ["silver", "gold"]]) {
      currentQuery = `filters=${encodeURIComponent(
        JSON.stringify([{ id: "layer", value, variant: "multiSelect", operator: "inArray", filterId: "a" }])
      )}`
      renderPage()
      const group = await screen.findByRole("group", { name: "Layer" })
      expect(within(group).queryAllByRole("button", { pressed: true })).toHaveLength(0)
      expect(await screen.findByRole("button", { name: "Remove Layer filter" })).toBeTruthy()
      cleanup()
    }
  })

  it("clears the layer with All", async () => {
    currentQuery = `filters=${encodeURIComponent(
      JSON.stringify([{ id: "layer", value: ["gold"], variant: "multiSelect", operator: "inArray", filterId: "a" }])
    )}`
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "All" }))
    expect(lastPushedFilter("layer")).toEqual([])
  })
})

describe("the Catalog's page frame", () => {
  it("offers Add data, to Sources, only to a person who may manage connectors", async () => {
    renderPage()
    await screen.findByText("Orders Silver")
    expect(screen.queryByRole("button", { name: "Add data" })).toBeNull()
    cleanup()

    permissions = ["catalog:read", "connector:manage"]
    renderPage()
    const add = await screen.findByRole("button", { name: "Add data" })
    expect(add.getAttribute("href")).toBe("/connectors")
  })

  it("has no description paragraph under the title", async () => {
    renderPage()
    await screen.findByText("Orders Silver")
    expect(screen.queryByText(/Every governed table and file/)).toBeNull()
  })

  it("keeps the table's settings in the page's ⋯: columns, export, refresh and layout", async () => {
    renderPage()
    await screen.findByText("Orders Silver")
    expect(screen.queryByRole("button", { name: "Table settings" })).toBeNull()
    fireEvent.pointerDown(screen.getByRole("button", { name: "More catalog actions" }))

    expect(await screen.findByRole("menuitem", { name: /Columns/ })).toBeTruthy()
    expect(screen.getByRole("menuitem", { name: /Export CSV/ })).toBeTruthy()
    expect(screen.getByRole("menuitem", { name: /Refresh data/ })).toBeTruthy()
    expect(screen.getByRole("menuitem", { name: /Reset layout/ })).toBeTruthy()
    // No table is dropped, so there is nothing to open.
    expect(screen.queryByRole("menuitem", { name: /Dropped tables/ })).toBeNull()
  })

  it("opens the dropped tables from the ⋯, with their count (DATA-55)", async () => {
    droppedTables = [
      {
        assetId: "orders",
        table: "bronze.orders",
        droppedBy: "Ada Lovelace",
        rowCount: 3,
        droppedAt: "2026-11-13T08:00:00Z",
        expiresAt: "2099-11-20T00:00:00Z",
      },
    ]
    renderPage()
    await screen.findByText("Orders Silver")
    fireEvent.pointerDown(screen.getByRole("button", { name: "More catalog actions" }))
    const item = await screen.findByRole("menuitem", { name: /Dropped tables/ })
    expect(item.textContent).toContain("1")
    fireEvent.click(item)

    expect(await screen.findByRole("dialog")).toBeTruthy()
    expect((await screen.findByRole("link", { name: /bronze\.orders/ })).getAttribute("href")).toBe(
      "/data/assets/orders"
    )
  })
})

describe("the Catalog's rows", () => {
  it("shows the table's address under its name, and no Namespace column", async () => {
    renderPage()
    expect(await screen.findByText("silver.orders")).toBeTruthy()
    expect(screen.queryByRole("columnheader", { name: "Namespace" })).toBeNull()
  })

  it("shows a dash for freshness that is not measured, explained once on the header", async () => {
    assetList = [{ ...ASSET, freshnessLagSeconds: null } as Asset]
    renderPage()
    await screen.findByText("Orders Silver")

    expect(screen.queryByText(/^Not measured$/, { ignore: ".sr-only" })).toBeNull()
    expect(screen.getByText("—")).toBeTruthy()
    expect(screen.getByRole("button", { name: /A dash means nothing measures/ })).toBeTruthy()
  })

  it("says nothing matches a search, with the one way to clear it", async () => {
    currentQuery = "search=kapal"
    assetList = []
    renderPage()

    expect(await screen.findByText("Nothing matches “kapal”")).toBeTruthy()
    expect(screen.getByRole("button", { name: "Clear search and filters" })).toBeTruthy()
  })

  it("says the catalog is empty when nothing narrows it", async () => {
    assetList = []
    renderPage()

    expect(await screen.findByText("Nothing in the catalog yet")).toBeTruthy()
  })
})

describe("the Catalog's refusal", () => {
  // QA LH-DATA-12-09 F1/F2, TC-1724 F1 (WEB-11): a refusal is not an outage,
  // and it is shown at once — the skeleton used to stay through the retries.
  it('shows "You don\'t have access" at once for a 403, with the permission, and asks once', async () => {
    const listCalls: string[] = []
    mock.restore()
    spyOn(globalThis, "fetch").mockImplementation((async (
      input: RequestInfo | URL
    ) => {
      const target = String(input)
      if (target.includes("/api/auth/me")) {
        return json({ id: "u1", name: "Reader", email: null, roles: [], permissions: [], tenants: [] })
      }
      if (target.startsWith("/api/catalog/query")) {
        listCalls.push(target)
        return json({ error: "permission_denied: catalog:read" }, 403)
      }
      if (target.startsWith("/api/catalog")) return json({ assets: [], namespaces: [NAMESPACE] })
      return json({ error: "not stubbed" }, 404)
    }) as unknown as typeof fetch)
    renderPage()

    expect(await screen.findByText("You don't have access")).toBeTruthy()
    expect(
      screen.getByText("This needs the catalog:read permission. Ask a workspace admin.")
    ).toBeTruthy()
    expect(screen.queryByText("Service unavailable")).toBeNull()
    expect(listCalls).toHaveLength(1)
  })
})
