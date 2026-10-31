// DATA-23: the Catalog page leaves for the Data Explorer by namespace id. A
// namespace has two values — the raw id the assets carry and the display
// label — and the page used the label in all three places it leaves for the
// explorer, so a labelled namespace ("Silver Layer" / `silver`) filtered
// nothing: the drawer's preview was empty and the button showed the
// explorer's empty namespace.
//
// The page is rendered whole, with `fetch` stubbed, so what is under test is
// the request the page sends and the link it puts on screen.
mock.module("next/navigation", () => ({
  usePathname: () => "/catalog",
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

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import { namespaceAssetsHref } from "@/lib/table-filter-link"
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
} as unknown as Asset

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** Every URL the page asked for, in order. */
let urls: string[] = []

function stubApi(namespace: CatalogNamespace = NAMESPACE) {
  urls = []
  spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const target = String(input)
    urls.push(target)
    if (target.startsWith("/api/catalog/query")) {
      // The API answers with the assets of the namespace the request names,
      // and a namespace is named by its id.
      const wanted = previewFilters().find((filter) => filter.id === "namespace")?.value
      const items = wanted === ASSET.namespace ? [ASSET] : []
      return json({ totalItems: items.length, totalPages: 1, pageSize: 5, page: 1, items })
    }
    if (target.startsWith("/api/catalog")) {
      return json({ assets: [], namespaces: [namespace] })
    }
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
}

function renderPage() {
  return render(
    <TableProviders>
      <CatalogPage />
    </TableProviders>
  )
}

/** The namespace row of the table, opened by a press on its name. */
async function openNamespace(name: string) {
  const cell = await screen.findByText(name)
  fireEvent.click(cell)
  return cell
}

/** The filters the preview asked for, as the API would read them. */
function previewFilters(): { id: string; value: string }[] {
  const query = urls.find((url) => url.startsWith("/api/catalog/query"))
  if (!query) throw new Error("the preview asked for nothing")
  const raw = new URL(query, "http://console.invalid").searchParams.get("filters")
  return JSON.parse(raw ?? "[]") as { id: string; value: string }[]
}

afterEach(() => {
  cleanup()
  mock.restore()
})

describe("the Catalog page's namespace links", () => {
  it("previews the namespace by its id, not by its display name", async () => {
    stubApi()
    renderPage()
    await openNamespace("Silver Layer")

    await waitFor(() => expect(urls.some((url) => url.startsWith("/api/catalog/query"))).toBe(true))
    expect(previewFilters()).toEqual([
      expect.objectContaining({ id: "namespace", value: NAMESPACE.id }),
    ])
  })

  it("links the drawer's button to the namespace id, not to its display name", async () => {
    stubApi()
    renderPage()
    await openNamespace("Silver Layer")

    // The button is a link to the explorer, so it carries the explorer's
    // filter — and that filter names the namespace by its id.
    const button = await screen.findByRole("button", { name: "Browse all 12 assets" })
    expect(button.getAttribute("href")).toBe(namespaceAssetsHref(NAMESPACE.id))
    expect(button.getAttribute("href")).not.toBe(namespaceAssetsHref(NAMESPACE.name))
  })

  it("links the drawer's other button to the namespace id too", async () => {
    // A namespace with few assets gets the other label on the same button.
    stubApi({ ...NAMESPACE, assetCount: 3 })
    renderPage()
    await openNamespace("Silver Layer")

    const button = await screen.findByRole("button", { name: "Browse in Data Explorer" })
    expect(button.getAttribute("href")).toBe(namespaceAssetsHref(NAMESPACE.id))
  })

  it("shows the namespace's assets in the drawer, because the preview asked by id", async () => {
    stubApi()
    renderPage()
    await openNamespace("Silver Layer")

    // The stub answers with the assets of the namespace the request named, so
    // this only holds when the request named the id.
    expect(await screen.findByRole("link", { name: /Orders Silver/ })).toBeTruthy()
    expect(screen.queryByText("No assets in this namespace yet.")).toBeNull()
  })
})
