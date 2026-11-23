import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { AssetDetail } from "@/services/contracts/assets"
import { AssetConnectDialog } from "./asset-connect-dialog"

const originalFetch = global.fetch

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

function asset(overrides: Partial<AssetDetail>): AssetDetail {
  return {
    id: "silver.orders",
    name: "Orders Silver",
    layer: "silver",
    ...overrides,
  } as unknown as AssetDetail
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

function stubGoldReadBack(body: unknown): void {
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input.toString()
    if (url.includes("/api/gold/export/mart_a")) return json(body)
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch
}

function openDialog(a: AssetDetail) {
  return render(<AssetConnectDialog asset={a} open onOpenChange={() => {}} />)
}

/** The REST tab's contents, which every zone sees. */
function expectRestContent() {
  expect(screen.getByRole("tab", { name: "Governed REST API" })).toBeTruthy()
  expect(screen.getByText(/POST \/api\/query\/run/)).toBeTruthy()
  expect(screen.getByText(/SELECT \* FROM .* LIMIT 100/)).toBeTruthy()
  expect(screen.getByText(/Authorization: Bearer <service-token>/)).toBeTruthy()
  expect(screen.getByText(/column masking/).textContent).toContain("row-level security (RLS)")
  expect(screen.getByText(/audit logging/)).toBeTruthy()
  const link = screen.getByRole("link", { name: "Open Service Identities" })
  expect(link.getAttribute("href")).toBe("/admin/service-identities")
  expect(screen.getByText(/query:read/)).toBeTruthy()
}

/** The REST tab is the only tab for a non-published asset — every zone. */
function expectGovernedRestOnly() {
  expectRestContent()
  expect(screen.queryByRole("tab", { name: "Iceberg REST Catalog" })).toBeNull()
}

describe("AssetConnectDialog — governed REST API for every zone", () => {
  it.each([
    ["bronze", "bronze.events"],
    ["silver", "silver.orders"],
  ] as const)("opens the REST tab for a %s table and hides Iceberg", (layer, id) => {
    const fetchMock = mock(async () => json({ error: "not stubbed" }, 404)) as unknown as typeof fetch
    global.fetch = fetchMock
    openDialog(asset({ id, layer }))
    expectGovernedRestOnly()
    // A non-Gold asset never asks the Gold routes.
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it("does not offer Iceberg for a Gold mart whose read-back says unpublished", async () => {
    stubGoldReadBack({ namespace: "gold", table: "mart_a", published: false })
    openDialog(asset({ id: "serving.mart_a", layer: "gold" }))
    await waitFor(() => expect(global.fetch).toHaveBeenCalled())
    await waitFor(() => expectGovernedRestOnly())
  })

  it("does not offer Iceberg when the read-back fails", async () => {
    stubGoldReadBack({ error: "catalog unavailable" })
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString()
      if (url.includes("/api/gold/export/mart_a")) return json({ error: "unavailable" }, 503)
      return json({ error: "not stubbed" }, 404)
    }) as unknown as typeof fetch
    openDialog(asset({ id: "serving.mart_a", layer: "gold" }))
    await waitFor(() => expect(global.fetch).toHaveBeenCalled())
    await waitFor(() => expectGovernedRestOnly())
  })
})

describe("AssetConnectDialog — Iceberg REST catalog for a published Gold table", () => {
  it("shows both tabs and the three client snippets with the exported-at note", async () => {
    stubGoldReadBack({
      namespace: "gold",
      table: "mart_a",
      published: true,
      formatVersion: 2,
      rowsInIceberg: 42,
      snapshotId: { value: 12345, measured: true },
      exportedAt: "2026-10-02T04:00:00Z",
    })
    openDialog(asset({ id: "serving.mart_a", layer: "gold" }))

    // The REST tab stays there for Gold too.
    expect(screen.getByRole("tab", { name: "Governed REST API" })).toBeTruthy()
    const icebergTab = await screen.findByRole("tab", { name: "Iceberg REST Catalog" })
    expectRestContent()

    fireEvent.click(icebergTab)
    expect(await screen.findByRole("tab", { name: "DuckDB" })).toBeTruthy()
    expect(screen.getByRole("tab", { name: "PyIceberg" })).toBeTruthy()
    expect(screen.getByRole("tab", { name: "Spark" })).toBeTruthy()
    expect(screen.getByText(/_exported_at/)).toBeTruthy()
    // Both appear in the prose and in the snippet's own comment.
    expect(screen.getAllByText(/\/api\/iceberg\/v1/).length).toBeGreaterThan(0)
    expect(screen.getAllByText(/iceberg:read/).length).toBeGreaterThan(0)

    // DuckDB is the default sub-tab; each other client's snippet is one click away.
    expect(screen.getByText(/ATTACH 'lakehouse' AS cat/)).toBeTruthy()
    fireEvent.click(screen.getByRole("tab", { name: "PyIceberg" }))
    expect(screen.getByText(/from pyiceberg.catalog import load_catalog/)).toBeTruthy()
    fireEvent.click(screen.getByRole("tab", { name: "Spark" }))
    expect(screen.getByText(/spark.sql.catalog.lakehouse.catalog-impl/)).toBeTruthy()

    for (const snippet of [
      screen.getByText(/ATTACH 'lakehouse' AS cat/).textContent ?? "",
      screen.getByText(/from pyiceberg.catalog import load_catalog/).textContent ?? "",
      screen.getByText(/spark.sql.catalog.lakehouse.catalog-impl/).textContent ?? "",
    ]) {
      expect(snippet).not.toContain(":8123")
      expect(snippet).not.toContain(":9000")
    }
  })
})
