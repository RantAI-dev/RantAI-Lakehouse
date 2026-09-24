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

import type { Pagination, PaginationQuery } from "./pagination"

/** `POST /api/catalog/{id}/access-request`'s body (WS7 item E2). */
export type RequestAccessInput = {
  permission: string
  reason: string
}

/**
 * Mirrors `lakehouse_store::agents::AccessGrant` — what an APPROVED
 * access request actually grants: one permission token, bounded by an
 * expiry.
 */
export type AccessGrant = {
  id: string
  approvalId: string
  userId: string
  permission: string
  grantedAt: string
  expiresAt: string
}

/**
 * `POST /api/catalog/access-requests/{id}/decide`'s response (WS7 item
 * E3). `grant` is present only for `status === "approved"` — a rejection
 * grants nothing (`lakehouse_store::agents::decide_access_request`
 * returns `Ok(None)` for `Decision::Rejected`).
 */
export type DecideAccessRequestResult = {
  status: "approved" | "rejected"
  grant?: AccessGrant
}

export interface AssetService {
  listAssets(filter: AssetFilter, signal?: AbortSignal): Promise<Asset[]>
  /**
   * One page of assets, with search/filter/sort/grouping applied on the
   * server (`GET /api/catalog/query`).
   *
   * Coexists with [`listAssets`] rather than replacing it: that method
   * returns the whole catalog and several screens still want exactly
   * that. This one backs the Advanced Data Table, where the query state
   * is richer than `AssetFilter` can express (multi-column sort, 14
   * filter operators, and/or joins) and the result set is paged.
   */
  listAssetsPage(
    query: PaginationQuery,
    signal?: AbortSignal
  ): Promise<Pagination<Asset>>
  getAsset(id: string, signal?: AbortSignal): Promise<AssetDetail>
  listNamespaces(signal?: AbortSignal): Promise<CatalogNamespace[]>
  /**
   * `POST /api/catalog/{id}/access-request` — ask for a permission not
   * already held on this catalog entry. `400`s server-side
   * (`routes::catalog::access_request`) if the caller already holds
   * `input.permission`, so the caller must only ever offer permissions
   * `hasPermission` has already excluded (see `requestableAccessPermissions`
   * in `@/lib/access-requests`).
   *
   * Optional (unlike this interface's other three methods) because
   * `src/services/mock/assets.ts` — a dead in-browser fixture this task
   * must not touch (see that file's own "dead fixture" comment
   * elsewhere in this module) — implements only the original three;
   * making this required would break that file's `: AssetService`
   * annotation for a fixture nothing wires up (`services/index.ts` never
   * assigns `assetService = mockAssetService`, only `clickhouseAssetService`
   * does). The real (ClickHouse-backed) client always implements it.
   */
  requestAccess?(
    catalogId: string,
    input: RequestAccessInput,
    signal?: AbortSignal
  ): Promise<void>
  /**
   * `POST /api/catalog/access-requests/{id}/decide` — a DEDICATED route
   * (N1 of the WS7 plan review): never reuses `agentService.decideApproval`,
   * which only ever decides a `kind = "tool_call"` approval. Refuses with
   * `403` (surfaced as a `ServiceError` with code `"permission_denied"`)
   * when the deciding principal is the same person who requested this
   * access (`routes::catalog::decide_access_request`, WS7 item E3).
   *
   * Optional for the same reason as `requestAccess` above.
   */
  decideAccessRequest?(
    approvalId: string,
    decision: "approved" | "rejected",
    comment: string | undefined,
    signal?: AbortSignal
  ): Promise<DecideAccessRequestResult>
}
