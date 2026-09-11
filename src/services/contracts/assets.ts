import type {
  CheckStatus,
  Classification,
  DataLayer,
  EngineCategory,
  Health,
  StorageTier,
} from "@/lib/status"
import type { Measured } from "@/lib/measured"

export type AssetType =
  | "table"
  | "view"
  | "iceberg-table"
  | "vector-dataset"
  | "external-source"
  | "knowledge-source"

export const ASSET_TYPE_LABEL: Record<AssetType, string> = {
  table: "Table",
  view: "View",
  "iceberg-table": "Open table",
  "vector-dataset": "Vector dataset",
  "external-source": "External source",
  "knowledge-source": "Knowledge source",
}

export type Asset = {
  id: string
  name: string
  namespace: string
  type: AssetType
  layer: DataLayer
  tier: StorageTier
  classification: Classification
  owner: string
  domain: string
  description: string
  format: string
  engine: EngineCategory
  // Silver rows are not queried; WS1 task 1.9 — null rather than a literal 0.
  rows: Measured
  // WS1 task 1.9 — not measured until WS2 reads Iceberg manifests.
  sizeBytes: Measured
  columnCount: number
  // WS1 task 1.9 — not measured until WS2 reads Iceberg snapshot timestamps.
  freshnessLagSeconds: Measured
  // WS1 task 1.9 — null in place of the empty-string placeholder for "unknown".
  lastUpdated: string | null
  health: Health
  residency: string
}

export type AssetColumn = {
  name: string
  dataType: string
  description?: string
  masked?: boolean
  classification?: Classification
}

export type AssetDetail = Asset & {
  schema: AssetColumn[]
  sample: Record<string, string>[]
  qualityChecks: {
    id: string
    name: string
    dimension: string
    status: CheckStatus
    lastRun: string
  }[]
  policySummary: { id: string; name: string; effect: string }[]
  // WS1 task 1.9 — nothing counts per-asset queries or users yet.
  usage: { queries7d: number; users7d: number; avgLatencyMs: number } | null
  recentQueries: { id: string; sql: string; user: string; at: string }[]
  dependents: { id: string; name: string; kind: string }[]
  changeHistory: { id: string; at: string; actor: string; summary: string }[]
  snapshots: { id: string; committedAt: string; operation: string; records: number }[]
  schemaVersions: { version: number; at: string; change: string }[]
  upstream: { id: string; name: string }[]
  downstream: { id: string; name: string }[]
  /**
   * WS1 task 1.9 — the API no longer emits this field (it named a policy
   * that exists nowhere). Kept optional only because the dead in-browser
   * fixture `src/services/mock/assets.ts` still sets it.
   */
  lifecyclePolicy?: string
  /**
   * WS2 §4 — the registry's own `table_name` for a Bronze asset, used to
   * try loading Iceberg snapshots for it (see `isIcebergCandidate` in
   * `@/lib/lakehouse-view`). The real API always sends this field (a
   * `string` or `null`) on the Bronze detail route; it is optional here
   * only because the dead in-browser fixture `src/services/mock/assets.ts`
   * does not set it. Consumers must treat `undefined` exactly like `null`.
   */
  tableName?: string | null
}

export type AssetFilter = {
  search?: string
  tier?: StorageTier | "all"
  layer?: DataLayer | "all"
  type?: AssetType | "all"
  classification?: Classification | "all"
}

export type CatalogNamespace = {
  id: string
  name: string
  description: string
  assetCount: number
  owner: string
  residency: string
  sourceEngine: string
}

export interface AssetService {
  listAssets(filter: AssetFilter, signal?: AbortSignal): Promise<Asset[]>
  getAsset(id: string, signal?: AbortSignal): Promise<AssetDetail>
  listNamespaces(signal?: AbortSignal): Promise<CatalogNamespace[]>
}
