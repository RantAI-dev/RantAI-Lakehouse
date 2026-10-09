// DATA-12: the certified / deprecated mark in the asset page's header. The
// page is rendered whole, with `fetch` stubbed: the mark must come from the
// asset the API sent, and an asset without one must show nothing.
mock.module("next/navigation", () => ({
  usePathname: () => "/data/assets/silver.orders",
  useParams: () => ({ assetId: "silver.orders" }),
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

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import type { AssetDetail } from "@/services/contracts/assets"
import { AssetDetailPage } from "./asset-detail-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

const ASSET: AssetDetail = {
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
  schema: [{ name: "id", dataType: "Int64" }],
  sample: [],
  qualityChecks: [],
  policySummary: [],
  usage: null,
  recentQueries: [],
  dependents: [],
  changeHistory: [],
  snapshots: [],
  schemaVersions: [],
  upstream: [],
  downstream: [],
} as unknown as AssetDetail

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

function stubApi(asset: Partial<AssetDetail>, permissions = ["catalog:read"]) {
  return spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    const path = String(input)
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Reader", email: null, roles: [], permissions, tenants: [] })
    }
    if (path === "/api/catalog/silver.orders") return json({ ...ASSET, ...asset })
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
}

function renderPage() {
  return render(
    <AuthProvider>
      <AssetDetailPage />
    </AuthProvider>
  )
}

describe("AssetDetailPage certification", () => {
  it.each([
    ["certified", "Certified"],
    ["deprecated", "Deprecated"],
  ])("shows the %s mark in the header", async (certification, label) => {
    stubApi({ certification })
    renderPage()
    expect(await screen.findByText("Orders Silver")).toBeTruthy()
    expect(screen.getByText(label)).toBeTruthy()
  })

  it("shows no mark for an asset that has none", async () => {
    stubApi({})
    renderPage()
    expect(await screen.findByText("Orders Silver")).toBeTruthy()
    await waitFor(() => expect(screen.queryByText("Certified")).toBeNull())
    expect(screen.queryByText("Deprecated")).toBeNull()
  })
})
