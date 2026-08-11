import type {
  CheckStatus,
  Classification,
  DataLayer,
  EngineCategory,
  Health,
  StorageTier,
} from "@/lib/status"

export type AssetType =
  | "table"
  | "view"
  | "iceberg-table"
  | "streaming-view"
  | "vector-dataset"
  | "external-source"
  | "knowledge-source"

export const ASSET_TYPE_LABEL: Record<AssetType, string> = {
  table: "Table",
  view: "View",
  "iceberg-table": "Open table",
  "streaming-view": "Streaming view",
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
  rows: number
  sizeBytes: number
  columnCount: number
  freshnessLagSeconds: number
  lastUpdated: string
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
  usage: { queries7d: number; users7d: number; avgLatencyMs: number }
  recentQueries: { id: string; sql: string; user: string; at: string }[]
  dependents: { id: string; name: string; kind: string }[]
  changeHistory: { id: string; at: string; actor: string; summary: string }[]
  snapshots: { id: string; committedAt: string; operation: string; records: number }[]
  schemaVersions: { version: number; at: string; change: string }[]
  upstream: { id: string; name: string }[]
  downstream: { id: string; name: string }[]
  lifecyclePolicy: string
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
