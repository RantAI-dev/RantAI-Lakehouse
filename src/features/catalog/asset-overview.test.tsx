// QS-10 (D5): the "Sensitive columns" card must not claim that a
// classification on its own masks a column. Rendering the overview directly
// keeps the assertion on the card, where a reader of the asset page looks.
import type { ReactNode } from "react"

mock.module("next/link", () => ({
  default: ({ href, children }: { href: string; children: ReactNode }) => (
    <a href={href}>{children}</a>
  ),
}))
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
mock.module("@/features/auth/auth-provider", () => ({
  useAuth: () => ({ hasPermission: () => true, hasRole: () => false }),
}))

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, expect, it, mock } from "bun:test"
import type { AssetDetail } from "@/services/contracts/assets"
import { AssetOverview } from "./asset-overview"

afterEach(() => {
  cleanup()
  mock.restore()
})

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

it("says a classification alone does not mask a column", () => {
  render(
    <AssetOverview
      asset={ASSET}
      iceberg={{ status: "loading", data: null, error: null, reload: () => {} }}
      lineage={{ status: "loading", data: null, error: null, reload: () => {} }}
      onNavigate={() => {}}
      onAssetChanged={() => {}}
    />
  )

  expect(
    screen.getByText(
      "Columns a policy masks, and columns classified confidential or above. A classification alone does not mask a column."
    )
  ).toBeTruthy()
})
