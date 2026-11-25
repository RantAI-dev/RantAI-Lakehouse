/**
 * Gold export surface — mirrors `routes::gold` exactly
 * (`rust/crates/lakehouse-api/src/routes/gold.rs`): marts (via
 * `GET /api/dashboard/fields`), the last export read back straight from
 * Iceberg, a manual "Export now" trigger, export run history, and the
 * count of outside tools that read the published table (DATA-5).
 */
import type { Measured } from "@/lib/measured"

/**
 * DATA-65: the publication zone. Silver reads `silver.<mart>` and writes
 * the `silver` Iceberg namespace; Gold reads `serving.<mart>` and writes
 * `gold`. Optional on responses so a pre-DATA-65 API shape still types.
 */
export type GoldZone = "silver" | "gold"

/** One `serving.*` ClickHouse mart, as `GET /api/dashboard/fields` lists it. */
export type GoldMart = {
  name: string
  rows: number
}

/** `POST /api/gold/export/{mart}` response — what the export just did. */
export type GoldExportResult = {
  zone?: GoldZone
  namespace: string
  table: string
  formatVersion: number
  rowsExported: number
  snapshotId: Measured
  exportedAt: string | null
  /**
   * The ONE `_exported_at` stamp this publish wrote on every row
   * (`DATA-6` M2), RFC 3339 with microseconds, or `null` on a zero-row
   * publish (no copy exists to identify).
   */
  copyExportedAt: string | null
  /**
   * What happened to the follow-up run that replaces
   * `gold.<table>_latest` with this copy (`DATA-6` D6/D13): `queued`, or
   * `not_started` when the orchestrator refused or could not be reached.
   * `null` on a zero-row publish.
   */
  latestCopy: { table: string; status: "queued" | "not_started" | string } | null
}

/**
 * `GET /api/gold/export/{mart}` response — the Gold Iceberg table read
 * straight back through `iceberg-rust`, independent of whatever the last
 * `POST` claimed.
 *
 * A mart never published answers 200 with `published: false` and every other
 * fact `null` (`DATA-49`, D11): "no export yet" is a normal state, not an
 * error, so the console reads it from this field. A failed request is a
 * failure and is shown as one, never as "Never exported".
 */
export type GoldReadBack = {
  zone?: GoldZone
  namespace: string
  table: string
  /** Whether the catalog holds this mart's published table at all. */
  published: boolean
  formatVersion: number | null
  rowsInIceberg: number | null
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
  zone?: GoldZone
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

/**
 * What the latest-copy table holds (DATA-6 D4), from the newest `success`
 * row of `console.gold_latest_copy_run`
 * (`gold_latest_copy::latest_copy_state` in `routes/gold.rs`).
 * `copyExportedAt` keeps microseconds — two publishes can fall in one
 * second — and is `null` only on a broken record, never on a healthy one.
 */
export type GoldLatestCopyHolds = {
  copyExportedAt: string | null
  rows: number | null
  replacedAt: string | null
}

/**
 * The newest latest-copy run of any status (DATA-6 D12). `reason` and
 * `cleanup` come from closed sets of fixed strings on the API side —
 * never exception text — widened per this repo's `"a" | "b" | string`
 * convention rather than treated as a closed union.
 */
export type GoldLatestCopyRun = {
  status: "success" | "failed" | "skipped" | "superseded" | string
  reason: string | null
  cleanup: string | null
  finishedAt: string | null
}

/**
 * The latest-copy block of a publication detail (DATA-6 D4): a second
 * Iceberg table, `gold.<table>_latest`, that always holds only the newest
 * copy, for tools that do not filter on export time. `holds`/`lastRun`
 * are `null` before the first update; the table's name is always there,
 * so a tool can be pointed at it ahead of time. The whole block is
 * `null` when this publish wrote no copy.
 */
export type GoldLatestCopy = {
  table: string
  holds: GoldLatestCopyHolds | null
  lastRun: GoldLatestCopyRun | null
}

/**
 * The mart's newest export run, when it FAILED (`DATA-49`). `failedAt` is when
 * that run started, RFC 3339, or `null` when it cannot be read; `reason` is the
 * sentence the API stored for it — the same one the failed response carried.
 */
export type GoldPublicationFailure = {
  failedAt: string | null
  reason: string
}

export type GoldPublication = {
  mart: string
  zone?: GoldZone
  enabled: boolean
  updatedAt: string | null
  lastChangedAt: string | null
  lastExportedAt: string | null
  changedSinceExport: boolean | null
  /**
   * `null` unless the mart's newest run failed, whatever the last successful
   * publish says (`DATA-49`, `publication_body` in `routes/gold.rs`).
   */
  lastFailure: GoldPublicationFailure | null
  canEdit: boolean
  trim?: GoldTrimSummary | null
  /** The latest-copy table's name, what it holds, its newest run (DATA-6). */
  latestCopy: GoldLatestCopy | null
}

export interface GoldService {
  listMarts(signal?: AbortSignal): Promise<GoldMart[]>
  getLastExport(mart: string, signal?: AbortSignal, zone?: GoldZone): Promise<GoldReadBack>
  triggerExport(mart: string, signal?: AbortSignal, zone?: GoldZone): Promise<GoldExportResult>
  listExportRuns(mart: string, signal?: AbortSignal, zone?: GoldZone): Promise<GoldExportRun[]>
  getConsumers(mart: string, signal?: AbortSignal, zone?: GoldZone): Promise<GoldConsumers>
  getPublication(mart: string, signal?: AbortSignal, zone?: GoldZone): Promise<GoldPublication>
  setPublication(mart: string, enabled: boolean, signal?: AbortSignal, zone?: GoldZone): Promise<GoldPublication>
}
