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
  /**
   * The age, in seconds, beyond which this asset is late, and whose word
   * that is: `"sla"` for an authored freshness SLA (used as written),
   * `"frequency"` for the registry's refresh cadence (one and a half
   * intervals). Absent or `null` when nothing says how often the asset
   * should refresh — its age is then shown without a verdict.
   */
  freshnessTargetSeconds?: number | null
  freshnessTargetSource?: "sla" | "frequency" | null
  // WS1 task 1.9 — null in place of the empty-string placeholder for "unknown".
  lastUpdated: string | null
  health: Health
  /**
   * The signals `health` rests on, one line each ("Late: written 8d 3h
   * ago, expected within 36h 00m", "1 of 2 quality checks passed"). Empty
   * when nothing measures the asset and it is `unknown`.
   */
  healthReasons?: string[]
  /**
   * `"rule"` when a classification rule names the asset or one of its
   * columns; `"default"` when none does and `classification` is the
   * deployment's default level.
   */
  classificationSource?: "rule" | "default"
  residency: string
  /**
   * Recorded by people in the console (`asset_annotation`), not by the
   * registry: who looks after the data day to day, and free-form tags.
   * Absent when not set. A set annotation `owner`/`description` already
   * replaces the registry's in `owner`/`description` above.
   */
  steward?: string | null
  tags?: string[]
}

/** What `PUT /api/catalog/{id}/annotation` stores; `null` clears a field. */
export type AssetAnnotation = {
  owner: string | null
  steward: string | null
  tags: string[]
  description: string | null
}

export type AssetColumn = {
  name: string
  dataType: string
  description?: string
  masked?: boolean
  classification?: Classification
  /**
   * When the connector that loads this Bronze table found the column gone
   * from its source (`SRC-8`): the column and its old values are kept.
   * `null` for a column the source still has. Absent on tables that no
   * connector loads (Silver, Serving), which carry no such fact.
   */
  inactiveSince?: string | null
}

export type AssetDetail = Asset & {
  schema: AssetColumn[]
  /**
   * The first rows, as the caller may read them. A cell is a string as
   * stored, or `null` for a `NULL`: an empty text is `""`, never `null`
   * (`routes/catalog.rs::governed_sample`).
   */
  sample: Record<string, string | null>[]
  /**
   * Checks that name this asset: verdicts a quality job recorded
   * (`origin: "observed"`) and rules people authored (`"rule"`). `status`
   * and `lastRun` are `null` for a rule nobody has run — never a
   * placeholder verdict.
   */
  qualityChecks: {
    id: string
    name: string
    /** Authored rules: the table the rule names (`bronze.orders`). */
    asset?: string
    dimension: string
    status: CheckStatus | null
    lastRun: string | null
    threshold?: string
    severity?: string
    origin?: "observed" | "rule"
    /** What the latest run measured, e.g. "97.2% not null". */
    value?: string | null
    /** Authored rules: whether the threshold can be run, and how to write one that can. */
    evaluable?: boolean
    hint?: string | null
  }[]
  /**
   * Policies whose condition binds one of this asset's tables
   * (`routes::catalog_governance`). `roles` and `rowFilter` are sent only
   * to a caller with `policy:read`; `mask` also to a caller the policy
   * applies to, who sees those columns as `***` anyway.
   */
  policySummary: {
    id: string
    name: string
    effect: string
    kind?: string
    /** `ready` is enforced; `draft` is not. */
    status?: string
    /** The table the policy binds, e.g. `silver.orders`. */
    table?: string
    /** Enforced on the caller's own reads. */
    appliesToYou?: boolean
    roles?: string[]
    mask?: string[]
    rowFilter?: string | null
  }[]
  /**
   * Queries that read this asset in the last seven days, by anyone:
   * how many, by how many people, and the average latency of the completed
   * ones. `null` only when the query history could not be read — a table
   * nobody queried is a measured zero.
   */
  usage: { queries7d: number; users7d: number; avgLatencyMs: number } | null
  /**
   * The caller's own recent queries on this asset. Other people's query
   * text is never sent: SQL can carry literals.
   */
  recentQueries: {
    id: string
    sql: string
    user: string
    at: string
    status?: string
    /** The audit event recorded for the run, when there is one. */
    auditEventId?: string | null
  }[]
  /**
   * Saved queries and dashboards whose SQL reads this asset
   * (`kind`: `"saved query"`, `"dashboard"`), each only for a caller who
   * may open it. Pipelines come from the lineage graph instead.
   */
  dependents: { id: string; name: string; kind: string; detail?: string }[]
  changeHistory: { id: string; at: string; actor: string; summary: string }[]
  snapshots: { id: string; committedAt: string; operation: string; records: number }[]
  /**
   * The versions of the table's schema, newest first. Where they come from
   * depends on the kind of table:
   * - `silver.*`/`serving.*`: recorded by the console itself (ADR 0015,
   *   `routes/schema_versions.rs`), because `ClickHouse` keeps only the current
   *   columns. A version is added each time a look at the table finds its
   *   ordered columns changed, so `at` is when the console *saw* the table in
   *   that shape, not when it changed, and the first version is dated by the
   *   first look. `change` is a sentence ("Added email (String)"; "First
   *   recorded with 4 columns" for the first); `current` is true on the newest
   *   only. Empty until the first look has recorded one.
   * - a Bronze Iceberg table: always `[]`. Its own versions are in the
   *   catalog's table detail (`useIcebergTable`, `LakehouseTableDetail`).
   */
  schemaVersions: { version: number; at: string; change: string; current: boolean }[]
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
  /**
   * The asset's annotation exactly as stored — what the edit form holds —
   * or `null` when nobody has annotated it. Absent from an older API build.
   */
  annotation?: AssetAnnotation | null
  /**
   * What the registry itself says, before any annotation replaced it: what
   * clearing an annotated field falls back to.
   */
  registry?: { owner: string | null; description: string | null }
  /**
   * The `<namespace>.<table>` key the lineage graph, policies and quality
   * rules name this asset by — `bronze.orders` for a Bronze dataset whose
   * `id` is the registry slug, the id itself for `silver.*`/`serving.*`.
   * `null` when the registry recorded no table, absent from an older API
   * build; fall back to `id` then.
   */
  tableKey?: string | null
  /**
   * The classification rules in force for this asset — the asset's own
   * first, then one per classified column — each removable by its id. An
   * older rule a newer one overrode is not listed. Absent from an older
   * API build.
   */
  classificationRules?: { id: string; column?: string; classification: Classification }[]
  /**
   * How ClickHouse holds a Silver or Gold asset's table: its engine and
   * keys, and what its active parts add up to. A field is `null` when it
   * could not be read or, for a key, when the table has none. Absent on a
   * Bronze asset, whose Iceberg table says this itself (`useIcebergTable`),
   * and from an older API build.
   */
  storage?: {
    /** `<database>.<table>`, as ClickHouse names it. */
    table: string
    engine: string | null
    partitionKey: string | null
    sortingKey: string | null
    parts: Measured
    partitions: Measured
    bytesOnDisk: Measured
    uncompressedBytes: Measured
    /** Every column of the table, system columns included. */
    tableColumns: number
  }
  /**
   * `true` when `sample` is empty because the caller lacks `query:read`
   * (the API withholds rows from `catalog:read`-only callers), not
   * because the asset has none. Absent from older API builds.
   */
  sampleRestricted?: boolean
  /**
   * What to query this asset as in Query Studio, resolved by the API
   * (`routes::catalog_source`): e.g. an Iceberg-only Bronze dataset is
   * `icecat_api.\`bronze.orders\`` on ClickHouse. Absent when nothing
   * readable backs the asset, or from an older API build.
   */
  queryTarget?: {
    engine: "clickhouse" | "trino"
    table: string
    /** The table name a policy binds to to govern reads of `table`. */
    policyTable?: string
  }
  /**
   * Registry facts the detail route passes through from
   * `bronze_meta.dataset_sync` (`catalog.rs`): how often the dataset is
   * refreshed, its unit, and the publisher's own classification. Each may
   * be an empty string; absent on assets that did not come from the sync.
   */
  _meta?: { frekuensi?: string; satuan?: string; klasifikasi?: string }
}

/**
 * One column of `GET /api/catalog/{id}/profile`
 * (`routes::catalog_profile`). Computed through the same policy rewrite as
 * Query Studio, so a masked column is profiled in its masked form.
 * `profiled: false` marks a column whose type (arrays, maps, …) or name
 * the profile does not aggregate; every stat is then absent.
 */
export type ColumnProfile = {
  name: string
  dataType: string
  profiled: boolean
  nullCount?: number | null
  /** 0–1 over `rowsProfiled`; `null` when no rows were read. */
  nullFraction?: number | null
  /** Approximate (`uniq`). */
  distinctCount?: number | null
  /** Numbers and dates only; free text gets none. */
  min?: string | null
  max?: string | null
  /** Most frequent values, only where the count is exact. */
  topValues?: { value: string; count: number }[]
}

/**
 * `supported: false` is an answer, not an error: e.g. an Iceberg-only
 * Bronze dataset has no ClickHouse table to profile, and `reason` says so.
 */
export type AssetProfile =
  | { supported: false; reason: string }
  | {
      supported: true
      /** The policy key of the table actually read (`silver.orders`, `bronze.orders`). */
      source: string
      /** Whether that table is a ClickHouse table or an Iceberg one read through ClickHouse. */
      sourceKind?: "clickhouse" | "iceberg"
      rowsProfiled: number
      rowLimit: number
      /** The row cap was reached — the table may hold more. */
      sampled: boolean
      /** The column cap was reached — some columns were left out. */
      columnsCapped: boolean
      columns: ColumnProfile[]
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
  /**
   * `GET /api/catalog/{id}/profile` — per-column stats. Needs `query:read`,
   * since the stats are read from the data itself. Optional for the same
   * dead-fixture reason as `requestAccess` below.
   */
  getAssetProfile?(id: string, signal?: AbortSignal): Promise<AssetProfile>
  /**
   * `GET /api/catalog/{id}/sample?limit=N` — more sample rows than the
   * detail body's five (the API caps the limit), masked and row-filtered
   * like any query the caller runs. Needs `query:read`.
   */
  getAssetSample?(
    id: string,
    limit: number,
    signal?: AbortSignal
  ): Promise<Record<string, string | null>[]>
  /**
   * `PUT /api/catalog/{id}/annotation` — replace the asset's annotation
   * (needs `catalog:write`). Optional for the same dead-fixture reason.
   */
  updateAnnotation?(id: string, input: AssetAnnotation, signal?: AbortSignal): Promise<void>
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
