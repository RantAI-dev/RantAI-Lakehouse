/**
 * Gold export surface — mirrors `routes::gold` exactly
 * (`rust/crates/lakehouse-api/src/routes/gold.rs`): marts (via
 * `GET /api/dashboard/fields`), the last export read back straight from
 * Iceberg, a manual "Export now" trigger, export run history, and the
 * count of outside tools that read the published table (DATA-5).
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
 * One outside tool that read a published Gold mart through the API's
 * Iceberg door (`GET /api/gold/export/{mart}/consumers`, DATA-5 D7).
 *
 * `name` is the identity's name at the time of the read, so a tool that
 * was later deleted keeps its past reads readable under the name it had
 * (ADR 0016, point 5). `reads` is the number of table loads counted over
 * the route's window; `lastReadAt` is when the last one happened.
 */
export type GoldConsumer = {
  identityId: string
  name: string
  reads: number
  lastReadAt: string
}

/**
 * `GET /api/gold/export/{mart}/consumers` response (DATA-5 D7/D9).
 *
 * The route reports one of two honest shapes, and never a third:
 * - `supported: true` with `consumers` listing every tool that read the
 *   mart in the window (`windowDays`, 7 today);
 * - `supported: false` with `consumers: null` and a `reason` naming what
 *   is missing — outside reading is off until an operator publishes
 *   storage at `OUTSIDE_S3_ENDPOINT` (D4).
 *
 * An empty `consumers` list means "nobody read it", which is a measured
 * zero. `consumers: null` with `supported: true` is not a shape the API
 * emits; the console renders it as unknown rather than as a zero, so a
 * broken contract never reads as "no readers".
 */
export type GoldConsumers = {
  mart: string
  supported: boolean
  windowDays?: number
  consumers: GoldConsumer[] | null
  reason?: string
}

/**
 * `GET /api/gold/export/{mart}/publication` response (gold-publish-per-mart
 * plan T7, `publication_body` in `routes/gold.rs`). `lastChangedAt` and
 * `lastExportedAt` are RFC 3339 strings (UTC, seconds granularity) or
 * `null` when not measurable.
 */
/**
 * Trim summary for a published Gold mart (DATA-2 D6).
 * From the newest `console.gold_trim_run` row; `null` means never trimmed.
 */
export type GoldTrimSummary = {
  status: "success" | "failed" | "nothing_to_trim" | string
  copiesKept: number | null
  oldestKeptExportedAt: string | null
  finishedAt: string
  reason: string | null
}

export type GoldPublication = {
  mart: string
  enabled: boolean
  updatedAt: string | null
  lastChangedAt: string | null
  lastExportedAt: string | null
  changedSinceExport: boolean | null
  canEdit: boolean
  trim?: GoldTrimSummary | null
}

export interface GoldService {
  listMarts(signal?: AbortSignal): Promise<GoldMart[]>
  getLastExport(mart: string, signal?: AbortSignal): Promise<GoldReadBack>
  triggerExport(mart: string, signal?: AbortSignal): Promise<GoldExportResult>
  listExportRuns(mart: string, signal?: AbortSignal): Promise<GoldExportRun[]>
  getConsumers(mart: string, signal?: AbortSignal): Promise<GoldConsumers>
  getPublication(mart: string, signal?: AbortSignal): Promise<GoldPublication>
  setPublication(mart: string, enabled: boolean, signal?: AbortSignal): Promise<GoldPublication>
}
