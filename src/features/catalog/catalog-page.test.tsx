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
// picker, and the links the namespace row puts on screen.

/** The query string the page reads; a test sets it before rendering. */
let currentQuery = ""

mock.module("next/navigation", () => ({
  usePathname: () => "/catalog",
  useSearchParams: () => new URLSearchParams(currentQuery),
  useRouter: () => ({
    push: () => {},
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

function stubApi() {
  spyOn(globalThis, "fetch").mockImplementation((async (
    input: RequestInfo | URL
  ) => {
    const target = String(input)
    if (target.startsWith("/api/catalog/query")) {
      return json({
        totalItems: ASSET_LIST.length,
        totalPages: 1,
        pageSize: 5,
        page: 1,
        items: ASSET_LIST,
      })
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
    <TableProviders>
      <CatalogPage />
    </TableProviders>
  )
}

/** Opens the toolbar's filter picker, which lists the filterable columns. */
async function openFilterPicker() {
  const trigger = await screen.findByRole("button", { name: "Filter" })
  fireEvent.click(trigger)
  return screen.findByPlaceholderText("Filter by...")
}

beforeEach(() => {
  currentQuery = ""
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

  it("says the filters still work when it hides a column", async () => {
    renderPage()

    await screen.findByRole("columnheader", { name: "Name" })
    expect(screen.getByText(/The filters still work\./)).toBeTruthy()
  })

  it("keeps the window's hide out of the layout it saves for the user", async () => {
    renderPage()
    await screen.findByRole("columnheader", { name: "Namespace" })

    // Hide a column the user can see, the way the header menu does. The
    // window's own hide must not ride along: it is the page's decision, not
    // the user's, and it would keep the column hidden after the window grows.
    fireEvent.pointerDown(screen.getByRole("button", { name: "Namespace" }))
    fireEvent.click(await screen.findByRole("menuitemcheckbox", { name: "Hide" }))

    const saved = JSON.parse(
      localStorage.getItem("app:table-layout:/catalog") ?? "{}"
    ) as { columnVisibility?: Record<string, boolean> }
    expect(saved.columnVisibility?.namespace).toBe(false)
    expect(saved.columnVisibility).not.toHaveProperty("owner")
    expect(saved.columnVisibility).not.toHaveProperty("tags")
  })
})

describe("the Catalog's namespace row", () => {
  /** The namespace row's link for one namespace, found by its display name. */
  async function namespaceChip(name: string) {
    const label = await screen.findByText(name)
    const link = label.closest("a")
    if (!link) throw new Error(`no link around ${name}`)
    return link
  }

  it("filters the list to the namespace id, not its display name (DATA-23)", async () => {
    renderPage()
    const link = await namespaceChip("Silver Layer")
    const filters = new URL(link.getAttribute("href") ?? "", "http://x").searchParams.get("filters") ?? ""

    expect(new URL(link.getAttribute("href") ?? "", "http://x").pathname).toBe("/catalog")
    expect(activeFilterValues(filters, "namespace")).toEqual([NAMESPACE.id])
    expect(link.getAttribute("href")).toBe(namespaceAssetsHref(NAMESPACE.id))
  })

  it("keeps the search term when a namespace is picked", async () => {
    currentQuery = "search=orders"
    renderPage()
    const link = await namespaceChip("Silver Layer")

    expect(new URL(link.getAttribute("href") ?? "", "http://x").searchParams.get("search")).toBe("orders")
  })

  it("describes the namespace the list is filtered to, with a way back to all of them", async () => {
    currentQuery = new URL(namespaceAssetsHref(NAMESPACE.id), "http://x").search.slice(1)
    renderPage()

    expect(await screen.findByText("Curated tables.")).toBeTruthy()
    expect(screen.getByText("id-jakarta")).toBeTruthy()
    expect(screen.getByText("ClickHouse + Iceberg")).toBeTruthy()
    const back = screen.getByRole("button", { name: "All namespaces" })
    expect(back.getAttribute("href")).toBe("/catalog")
  })

  it("says the namespaces could not load, and still lists the assets", async () => {
    namespacesFail = true
    renderPage()

    expect(await screen.findByText("Could not load namespaces.")).toBeTruthy()
    const row = screen.getByText("Could not load namespaces.").parentElement
    expect(row ? within(row).getByRole("button", { name: "Retry" }) : null).toBeTruthy()
    expect(await screen.findByText("Orders Silver")).toBeTruthy()
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
