/**
 * Pure view logic for the Lakehouse tables/table-detail pages and the
 * catalog Snapshots tab's Iceberg branch (WS2 §4). Kept out of the
 * page components so it can be unit-tested with `bun test` — the
 * component-test harness (H0) does not exist on this branch.
 */
import { formatRelativeTime } from "./format"
import type {
  LakehouseMaintenance,
  LakehouseSnapshot,
} from "@/services/contracts/lakehouse"

export function lakehouseWarehousesUrl(): string {
  return "/api/lakehouse/warehouses"
}

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
 * An asset can only have Iceberg snapshots surfaced when it is a Bronze
 * asset AND the registry recorded a (possibly Iceberg, possibly not —
 * see the module comment in `catalog.rs`) `tableName` for it. The
 * actual existence check happens server-side via a 404 on
 * `getTableDetail`.
 */
export function isIcebergCandidate(asset: {
  layer: string
  tableName?: string | null
}): boolean {
  return asset.layer === "bronze" && typeof asset.tableName === "string" && asset.tableName.length > 0
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
