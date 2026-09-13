/**
 * Read-only Iceberg warehouse/namespace/table surface (WS2 §4). Mirrors
 * `rust/crates/lakehouse-api/src/routes/lakehouse.rs`'s JSON bodies field
 * for field — every numeric value the backend cannot always measure is a
 * `Measured` rather than a fabricated `0`, and every optional string is
 * `string | null`, never an empty-string placeholder.
 *
 * There is no `health` field anywhere in this contract — this workstream
 * never populates one; `health` continues to live only on
 * `contracts/assets.ts`'s catalog contract.
 */
import type { Measured } from "@/lib/measured"

export type LakehouseWarehouse = {
  id: string
  name: string
  /**
   * Reported by the Management API only, which this service's reader
   * identity cannot call — always `null` today, kept typed loosely (not
   * `string | null`) so a future real value isn't forced into the wrong
   * shape.
   */
  storageProfile: unknown
  /**
   * Nothing on this service proves Lakekeeper actually vends credentials
   * for this warehouse's storage profile — always `null` today.
   */
  credentialVending: unknown
  reachable: boolean
}

export type LakehouseNamespace = {
  name: string
  /** `null` when the per-namespace table listing did not finish in budget. */
  tableCount: Measured
}

export type LakehouseTableSummary = {
  namespace: string
  name: string
  /** `null` when the per-table load did not finish in budget or failed. */
  formatVersion: Measured
  /**
   * A 64-bit Iceberg snapshot id, sent as a string because it routinely
   * exceeds `Number.MAX_SAFE_INTEGER` and a JSON number would be rounded
   * by `JSON.parse`. Not a `Measured` — it is an identifier, not a metric.
   */
  currentSnapshotId: string | null
  lastUpdatedAt: string | null
  fileCount: Measured
  recordCount: Measured
  totalBytes: Measured
}

export type LakehouseSchemaField = {
  id: number
  name: string
  // Iceberg primitive/nested type name, e.g. "long", "string", "struct<...>".
  type: string
  required: boolean
}

export type LakehousePartitionField = {
  sourceId: number
  transform: string
  name: string
}

export type LakehouseSnapshotSummary = {
  addedRecords: Measured
  deletedRecords: Measured
  totalRecords: Measured
  totalDataFiles: Measured
}

export type LakehouseSnapshot = {
  /** A 64-bit Iceberg snapshot id — a string, for the reason documented on `currentSnapshotId` above. */
  id: string
  parentId: string | null
  timestampMs: number
  // Iceberg snapshot operation — known values widened with `| string` since
  // the catalog is free to report an operation this contract doesn't list.
  operation: "append" | "replace" | "overwrite" | "delete" | string
  summary: LakehouseSnapshotSummary
}

export type LakehouseTableStats = {
  fileCount: Measured
  /** Needs a manifest read this workstream does not do — always `null`. */
  smallFileCount: Measured
  smallFileThresholdBytes: Measured
  recordCount: Measured
  totalBytes: Measured
  snapshotCount: number
  metadataLogCount: number
}

export type LakehouseTableDetail = {
  schema: LakehouseSchemaField[]
  partitionSpec: LakehousePartitionField[]
  properties: Record<string, string>
  /** Oldest first, exactly as the API returns them. */
  snapshots: LakehouseSnapshot[]
  stats: LakehouseTableStats
}

/**
 * One `bronze_meta.maintenance_run` row, mirroring
 * `routes/governance.rs::maintenance_run_row_json` exactly — every counter
 * is a `ClickHouse`-stringified number, not a JS number, matching what the
 * backend actually serializes.
 */
export type LakehouseMaintenanceRun = {
  tableName: string
  runAt: string
  dryRun: { deletedDataFiles: string; deletedManifestFiles: string }
  applied: { deletedDataFiles: string; deletedManifestFiles: string }
  skippedVerbs: string
}

/**
 * One `bronze_meta.maintenance_verb_run` row, mirroring
 * `routes/lakehouse.rs::maintenance_verb_run_row_json` exactly. `outcome`
 * is widened with `| string` since the job can record an outcome this
 * contract doesn't yet list — e.g. `"refused"` when `expire_snapshots` hits
 * the 7-day retention floor, which a caller must still be able to display
 * rather than have swallowed by a type mismatch.
 */
export type LakehouseVerbRun = {
  verb: string
  engine: string
  outcome: "applied" | "refused" | "skipped" | "failed" | string
  detail: string
  runAt: string
}

export type LakehouseMaintenance = {
  namespace: string
  tableName: string
  configured: boolean
  snapshotsToKeep: Measured
  orphanAgeHours: Measured
  /**
   * `false` when unconfigured — the maintenance job's real default behavior
   * skips compaction, not "unknown".
   */
  compactSmallFiles: boolean
  schedule: string | null
  lastRun: LakehouseMaintenanceRun | null
  /** Additive: newest recorded run of each verb, oldest-first is not implied. */
  lastVerbRuns: LakehouseVerbRun[]
}

/**
 * `POST /api/lakehouse/tables/{ns}/{table}/maintenance`'s body — camelCase
 * on the wire, matching `MaintenancePolicyBody` in
 * `routes/lakehouse.rs`. `snapshotsToKeep`/`orphanAgeHours` of `null` clear
 * that field to the maintenance job's default; setting either below 1 is
 * rejected both here (`validateMaintenancePolicyForm`) and by the route.
 */
export type MaintenancePolicyInput = {
  snapshotsToKeep: number | null
  orphanAgeHours: number | null
  compactSmallFiles: boolean
  schedule: string | null
}

/**
 * The route's POST response — the GET `LakehouseMaintenance` shape minus
 * `lastRun`/`lastVerbRuns`, which a save does not recompute (no
 * `ClickHouse` round trip happens on write; see `set_maintenance_policy`'s
 * doc comment).
 */
export type MaintenancePolicyResult = Omit<LakehouseMaintenance, "lastRun" | "lastVerbRuns">

/**
 * One `bucket_name`'s newest reading from `bronze_meta.capacity_snapshot`,
 * mirroring `routes/lakehouse.rs::capacity_body`'s `buckets[]` entries.
 * `bytes`/`objects` are plain numbers, not `Measured` — the backend's
 * `num_or_zero` already defaults an unparseable or missing column to `0`
 * before this ever reaches the wire, so there is no "not measured" state
 * left to represent here.
 */
export type LakehouseCapacityBucket = {
  name: string
  bytes: number
  objects: number
  measuredAt: string
}

/**
 * `GET /api/lakehouse/capacity`'s body, mirroring `capacity_body` exactly.
 * `growth7d` is `Measured` (not a plain `number`) because it really can be
 * absent by design: `null` whenever no reading falls within the ±12 hour
 * window around `latest - 7 days` (fewer than ~8 days of history, or a
 * missed daily run) — see `capacity_body`'s doc comment. Render it with
 * `fmtMeasured`, never `?? 0`.
 */
export type LakehouseCapacity = {
  buckets: LakehouseCapacityBucket[]
  clickhouse: { bytesOnDisk: number }
  growth7d: Measured
}

export interface LakehouseService {
  listWarehouses(signal?: AbortSignal): Promise<LakehouseWarehouse[]>
  getCapacity(signal?: AbortSignal): Promise<LakehouseCapacity>
  listNamespaces(warehouse?: string, signal?: AbortSignal): Promise<LakehouseNamespace[]>
  listTables(
    namespace: string,
    warehouse?: string,
    signal?: AbortSignal
  ): Promise<LakehouseTableSummary[]>
  getTableDetail(namespace: string, table: string, signal?: AbortSignal): Promise<LakehouseTableDetail>
  getMaintenance(namespace: string, table: string, signal?: AbortSignal): Promise<LakehouseMaintenance>
  setMaintenancePolicy(
    namespace: string,
    table: string,
    input: MaintenancePolicyInput,
    signal?: AbortSignal
  ): Promise<MaintenancePolicyResult>
}
