// DATA-24: the Owner and Tags filters vanished on a laptop-width window.
// The columns were dropped from the table's column list, and a filter lives
// on a column, so the filters went with them — the one thing a narrow
// window took away was the one thing the narrow window was for.
//
// The columns are hidden now, not removed: they stay in the column list, so
// the toolbar's filter picker still offers them at every width, while the
// table itself leaves the room they would take.
//
// The page is rendered whole, with `fetch` stubbed, so what is under test is
// what the user is offered: the columns on screen, and the filters in the
// picker.
mock.module("next/navigation", () => ({
  usePathname: () => "/data",
  useSearchParams: () => new URLSearchParams(""),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import type { Asset } from "@/services/contracts/assets"
import { DataExplorerPage } from "./data-explorer-page"

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
      <DataExplorerPage />
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

describe("the Data Explorer's columns at a narrow window", () => {
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
      localStorage.getItem("app:table-layout:/data") ?? "{}"
    ) as { columnVisibility?: Record<string, boolean> }
    expect(saved.columnVisibility?.namespace).toBe(false)
    expect(saved.columnVisibility).not.toHaveProperty("owner")
    expect(saved.columnVisibility).not.toHaveProperty("tags")
  })
})
