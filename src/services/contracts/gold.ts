/**
 * Gold export surface — mirrors `routes::gold` exactly
 * (`rust/crates/lakehouse-api/src/routes/gold.rs`): marts (via
 * `GET /api/dashboard/fields`), the last export read back straight from
 * Iceberg, a manual "Export now" trigger, export run history, and a
 * best-effort consumers count.
 */
import type { Measured } from "@/lib/measured"

/** One `serving.*` ClickHouse mart, as `GET /api/dashboard/fields` lists it. */
export type GoldMart = {
  name: string
  rows: number
}

/** `POST /api/gold/export/{mart}` response — what the export just did. */
export type GoldExportResult = {
  namespace: string
  table: string
  formatVersion: number
  rowsExported: number
  snapshotId: Measured
  exportedAt: string | null
}

/**
 * `GET /api/gold/export/{mart}` response — the Gold Iceberg table read
 * straight back through `iceberg-rust`, independent of whatever the last
 * `POST` claimed. 500s when the table has never been exported; the client
 * classifies that as `ServiceError` for the page to render as "Never
 * exported" rather than a generic error.
 */
export type GoldReadBack = {
  namespace: string
  table: string
  formatVersion: number
  rowsInIceberg: number
  snapshotId: Measured
  exportedAt: string | null
}

/**
 * One row from `GET /api/gold/exports?mart=`, mirroring
 * `gold_export_history::GoldExportRunRow`. `status` is a plain `String` on
 * the Rust side (not a serde enum), so it is widened per this repo's
 * `"a" | "b" | string` convention rather than treated as a closed union.
 * `rowsExported`/`formatVersion`/`snapshotId` use `Measured` because a
 * failed run before any row was read genuinely never produced a count —
 * distinct from a run that produced `0`.
 */
export type GoldExportRun = {
  id: string
  status: "success" | "failed" | string
  rowsExported: Measured
  formatVersion: Measured
  snapshotId: Measured
  error: string | null
  triggeredBy: string
  startedAt: string
  finishedAt: string
}

/**
 * `GET /api/gold/export/{mart}/consumers` response. This route ships as
 * an honest stub (no Trino query-history correlation exists yet — see
 * `routes::gold::consumers`'s doc comment): always `supported: false` with
 * a `reason` explaining why, never a silently-empty `consumers` list,
 * which would read as "this mart has zero consumers" — a different, false
 * claim.
 */
export type GoldConsumers = {
  mart: string
  consumers: unknown[] | null
  supported: boolean
  reason?: string
}

export interface GoldService {
  listMarts(signal?: AbortSignal): Promise<GoldMart[]>
  getLastExport(mart: string, signal?: AbortSignal): Promise<GoldReadBack>
  triggerExport(mart: string, signal?: AbortSignal): Promise<GoldExportResult>
  listExportRuns(mart: string, signal?: AbortSignal): Promise<GoldExportRun[]>
  getConsumers(mart: string, signal?: AbortSignal): Promise<GoldConsumers>
}
