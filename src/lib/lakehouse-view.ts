/**
 * Pure view logic for the Lakehouse tables/table-detail pages and the
 * catalog Snapshots tab's Iceberg branch (WS2 §4). Kept out of the
 * page components so it can be unit-tested with `bun test` — the
 * component-test harness (H0) does not exist on this branch.
 */
import { formatBytes, formatRelativeTime } from "./format"
import type { Measured } from "./measured"
import type {
  LakehouseCompactionStatus,
  LakehouseCompactionTable,
  LakehouseMaintenance,
  LakehouseSnapshot,
} from "@/services/contracts/lakehouse"

export function lakehouseNamespacesUrl(warehouse?: string): string {
  return warehouse === undefined
    ? "/api/lakehouse/namespaces"
    : `/api/lakehouse/namespaces?warehouse=${encodeURIComponent(warehouse)}`
}

export function lakehouseTablesUrl(namespace: string, warehouse?: string): string {
  const parts = [`namespace=${encodeURIComponent(namespace)}`]
  if (warehouse !== undefined) parts.push(`warehouse=${encodeURIComponent(warehouse)}`)
  return `/api/lakehouse/tables?${parts.join("&")}`
}

export function lakehouseTableDetailUrl(namespace: string, table: string): string {
  return `/api/lakehouse/tables/${encodeURIComponent(namespace)}/${encodeURIComponent(table)}`
}

export function lakehouseMaintenanceUrl(namespace: string, table: string): string {
  return `${lakehouseTableDetailUrl(namespace, table)}/maintenance`
}

export function lakehouseCapacityUrl(): string {
  return "/api/lakehouse/capacity"
}

export function lakehouseCompactionStatusUrl(): string {
  return "/api/lakehouse/compaction-status"
}

/**
 * Count at which measured small files need compaction (OPS-21). Mirrors the
 * backend's `NEEDS_COMPACTION_FILE_COUNT`: the G3 measurement of 20 small
 * files per day partition planning ~15-20x worse than a one-file control
 * (`docs/plans/G3-RESULT.md`, ADR 0009). The System-status endpoint also
 * sends this number; the table-detail response does not, so the console owns
 * the same documented constant for that surface.
 */
export const NEEDS_COMPACTION_FILE_COUNT = 20

/**
 * The honest reason compaction cannot happen here. Names the `trino`
 * Compose profile explicitly, because that is the fix a reader can act on;
 * the other three engines genuinely cannot bin-pack small files (OPS-21).
 */
const COMPACTION_FIX =
  "Small files are merged only by Trino in this deployment: pyiceberg and the iceberg crate cannot compact, and ClickHouse OPTIMIZE does not bin-pack. Start Compose with --profile trino (the trino profile) to enable compaction."

/** Table-page warning copy, built from a real measured count. */
export function tableCompactionWarningMessage(smallFileCount: number, thresholdBytes?: Measured): string {
  const size =
    thresholdBytes === null || thresholdBytes === undefined ? "" : ` below ${formatBytes(thresholdBytes)}`
  return `${smallFileCount} live data files${size} in this table's current snapshot need compaction, but nothing here can merge them while the trino profile is off. ${COMPACTION_FIX}`
}

/** System-status warning copy for a set of affected tables. */
export function systemCompactionWarningMessage(tableCount: number): string {
  return `${tableCount} ${tableCount === 1 ? "table has" : "tables have"} measured small files that need compaction, but nothing here can merge them while the trino profile is off. ${COMPACTION_FIX}`
}

/**
 * A table page warns only from a MEASURED count at or above the need count
 * AND a confirmed-unavailable Trino (OPS-21). `null` (unmeasured), a
 * below-threshold count, and `compactionAvailable !== false` (available, or
 * an older API that did not send the flag) are all silent — never infer a
 * warning from an absent measurement.
 */
export function tableNeedsCompactionWarning(table: {
  smallFileCount: Measured
  compactionAvailable?: boolean
}): boolean {
  return (
    table.smallFileCount !== null &&
    table.smallFileCount >= NEEDS_COMPACTION_FILE_COUNT &&
    table.compactionAvailable === false
  )
}

/**
 * The Bronze tables the System-status warning lists (OPS-21). The backend
 * already filters by its need count, but re-checking here means a future API
 * change cannot make the console warn off an unmeasured or below-threshold
 * row, and the `compactionAvailable === false` guard keeps the warning
 * silent when Trino is on. Only measured rows are ever returned.
 */
export function tablesNeedingCompaction(
  status: Pick<LakehouseCompactionStatus, "compactionAvailable" | "tables" | "needsCompactionFileCount">
): LakehouseCompactionTable[] {
  if (status.compactionAvailable !== false) return []
  return status.tables
    .filter((t) => t.smallFileCount >= status.needsCompactionFileCount)
    .sort((a, b) => a.namespace.localeCompare(b.namespace) || a.name.localeCompare(b.name))
}

/**
 * Renders `growth7d` (a signed byte delta — shrinkage is a real, negative
 * measurement) with an explicit `+`/`-` sign, built on top of `formatBytes`
 * rather than a second byte formatter: `formatBytes` itself only accepts
 * non-negative magnitudes, so the sign is split off before it, not
 * reimplemented inside it.
 */
export function formatSignedBytes(bytes: number): string {
  const sign = bytes < 0 ? "-" : "+"
  return `${sign}${formatBytes(Math.abs(bytes))}`
}

/** Link to the table-detail page for a given namespace/table pair. */
export function lakehouseTableHref(namespace: string, table: string): string {
  return `/lakehouse/tables/${encodeURIComponent(namespace)}/${encodeURIComponent(table)}`
}

/**
 * Returns a NEW array of `snapshots` sorted newest first — the API returns
 * them oldest first. Never mutates `snapshots`.
 */
export function snapshotsNewestFirst(snapshots: LakehouseSnapshot[]): LakehouseSnapshot[] {
  return [...snapshots].sort((a, b) => b.timestampMs - a.timestampMs)
}

/**
 * An asset can only have Iceberg snapshots surfaced when the registry
 * recorded a `tableName` for it AND it is either a Bronze asset or typed
 * `"iceberg-table"`. The latter matters because the detail route files
 * every non-curated registry dataset under layer `raw` while still typing
 * it `"iceberg-table"` (`catalog.rs`), and those have real Bronze tables
 * too. Neither signal proves the table exists (see the module comment in
 * `catalog.rs`) — that check happens server-side via a 404 on
 * `getTableDetail`.
 */
export function isIcebergCandidate(asset: {
  layer: string
  type?: string
  tableName?: string | null
}): boolean {
  const icebergish = asset.layer === "bronze" || asset.type === "iceberg-table"
  return icebergish && typeof asset.tableName === "string" && asset.tableName.length > 0
}

/** ISO-8601 UTC string from an Iceberg epoch-millisecond timestamp, for `formatRelativeTime`. */
export function msToIso(ms: number): string {
  return new Date(ms).toISOString()
}

/** Relative-time label for a snapshot's `timestampMs`. */
export function snapshotRelativeTime(timestampMs: number, now?: number): string {
  return formatRelativeTime(msToIso(timestampMs), now)
}

const UNCONFIGURED_SUMMARY =
  "No policy — default maintenance (orphan-file removal only)"

/** Human summary of a table's maintenance policy, for the read-only detail-page section. */
export function maintenanceSummary(m: LakehouseMaintenance): string {
  if (!m.configured) return UNCONFIGURED_SUMMARY

  const parts: string[] = []
  parts.push(
    m.snapshotsToKeep === null
      ? "keep snapshots: not set"
      : `keep the ${m.snapshotsToKeep} newest snapshots`
  )
  parts.push(
    m.orphanAgeHours === null
      ? "orphan-file age: not set"
      : `remove orphan files older than ${m.orphanAgeHours}h`
  )
  parts.push(m.compactSmallFiles ? "compact small files" : "no small-file compaction")
  if (m.schedule !== null) parts.push(`schedule: ${m.schedule}`)
  return parts.join(", ")
}
