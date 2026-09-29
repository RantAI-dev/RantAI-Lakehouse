"use client"

import * as React from "react"
import Link from "next/link"
import { MetadataList } from "@/components/patterns/metadata-list"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatDateTime, formatNumber } from "@/lib/format"
import { lakehouseTableHref, msToIso, snapshotRelativeTime } from "@/lib/lakehouse-view"
import { fmtMeasured } from "@/lib/measured"
import { cn } from "@/lib/utils"
import { lakehouseService } from "@/services"
import type {
  LakehouseMaintenance,
  LakehouseSnapshot,
  LakehouseTableDetail,
} from "@/services/contracts/lakehouse"

/**
 * Every Iceberg candidate lives in this namespace — the same assumption
 * `IcebergSnapshots` in `asset-detail-tabs.tsx` makes (dlt writes Bronze
 * with `dataset_name="bronze"`, see the module comment in `catalog.rs`).
 */
const BRONZE_NAMESPACE = "bronze"

/** `day(_ingested_at)` — how Iceberg itself writes a partition field. */
function partitionLabel(detail: LakehouseTableDetail) {
  return detail.partitionSpec.map((p) => {
    const source = detail.schema.find((f) => f.id === p.sourceId)?.name ?? `#${p.sourceId}`
    return p.transform === "identity" ? source : `${p.transform}(${source})`
  })
}

function maintenanceLabel(m: LakehouseMaintenance) {
  if (!m.configured) return "Not configured"
  return m.schedule ? `Scheduled · ${m.schedule}` : "Configured, no schedule"
}

/**
 * Row count after each snapshot, oldest first. A snapshot whose summary
 * did not report `totalRecords` is left out rather than drawn as zero.
 */
function volumePoints(snapshots: LakehouseSnapshot[]) {
  return [...snapshots]
    .sort((a, b) => a.timestampMs - b.timestampMs)
    .flatMap((s) =>
      s.summary.totalRecords === null
        ? []
        : [{ id: s.id, t: s.timestampMs, rows: s.summary.totalRecords, op: s.operation }]
    )
}

type VolumePoint = ReturnType<typeof volumePoints>[number]

const CHART_W = 600
const CHART_H = 96

/**
 * Row count across snapshots: one series, so no legend — the card title
 * names it. The y-axis starts at zero because this is a magnitude, and a
 * cropped axis would turn a 1% append into a cliff. The Snapshots tab is
 * this chart's table view.
 */
function VolumeTrend({ points }: { points: VolumePoint[] }) {
  const [hover, setHover] = React.useState<number | null>(null)
  const max = Math.max(...points.map((p) => p.rows), 1)
  const t0 = points[0].t
  const span = Math.max(points[points.length - 1].t - t0, 1)
  // Fractions of the plot, so HTML overlays line up with the stretched SVG.
  const fx = (p: VolumePoint) => (points.length === 1 ? 0.5 : (p.t - t0) / span)
  const fy = (p: VolumePoint) => 1 - p.rows / max
  const path = points
    .map((p, i) => `${i === 0 ? "M" : "L"}${fx(p) * CHART_W},${fy(p) * CHART_H}`)
    .join(" ")
  const active = hover === null ? null : points[hover]
  const last = points[points.length - 1]

  return (
    <div>
      <div className="mb-2 flex items-baseline gap-2">
        <span className="text-2xl font-semibold tabular-nums">{formatNumber(last.rows)}</span>
        <span className="text-xs text-muted-foreground">rows now</span>
      </div>
      <div
        className="relative text-primary"
        style={{ height: CHART_H }}
        onMouseLeave={() => setHover(null)}
      >
        <svg
          viewBox={`0 0 ${CHART_W} ${CHART_H}`}
          preserveAspectRatio="none"
          className="absolute inset-0 size-full overflow-visible"
          aria-hidden
        >
          <line
            x1={0}
            x2={CHART_W}
            y1={CHART_H}
            y2={CHART_H}
            className="stroke-border"
            vectorEffect="non-scaling-stroke"
          />
          {points.length > 1 ? (
            <path
              d={path}
              fill="none"
              stroke="currentColor"
              strokeWidth={2}
              strokeLinejoin="round"
              strokeLinecap="round"
              vectorEffect="non-scaling-stroke"
            />
          ) : null}
        </svg>
        {/* Markers in HTML so they stay round on the stretched plot. */}
        {points.map((p, i) => (
          <span
            key={p.id}
            className={cn(
              "pointer-events-none absolute size-2 -translate-x-1/2 -translate-y-1/2 rounded-full bg-current ring-2 ring-card",
              hover === i && "size-2.5"
            )}
            style={{ left: `${fx(p) * 100}%`, top: `${fy(p) * 100}%` }}
          />
        ))}
        {active ? (
          <>
            <span
              className="pointer-events-none absolute inset-y-0 w-px bg-border"
              style={{ left: `${fx(active) * 100}%` }}
            />
            <div
              className={cn(
                "pointer-events-none absolute -top-2 z-10 -translate-y-full whitespace-nowrap rounded-md border border-border bg-popover px-2 py-1 text-xs text-popover-foreground shadow-sm",
                fx(active) > 0.7 ? "-translate-x-full" : fx(active) < 0.3 ? "" : "-translate-x-1/2"
              )}
              style={{ left: `${fx(active) * 100}%` }}
            >
              <div className="font-medium tabular-nums">{formatNumber(active.rows)} rows</div>
              <div className="text-muted-foreground">
                {active.op} · {formatDateTime(msToIso(active.t))}
              </div>
            </div>
          </>
        ) : null}
        {/* Hit bands: each point owns the horizontal slice nearest to it. */}
        <div className="absolute inset-0 flex">
          {points.map((p, i) => (
            <div
              key={p.id}
              className="h-full flex-1"
              onMouseEnter={() => setHover(i)}
              aria-label={`${formatNumber(p.rows)} rows, ${formatDateTime(msToIso(p.t))}`}
            />
          ))}
        </div>
      </div>
      {points.length === 1 ? (
        <p className="mt-2 text-xs text-muted-foreground">
          One snapshot so far — the trend appears after the next load.
        </p>
      ) : null}
    </div>
  )
}

/**
 * Physical side of an Iceberg-backed asset: files, bytes, partitioning,
 * maintenance, and row count over snapshots. Its own component because
 * `useService` cannot be called conditionally in the overview.
 */
export function AssetStorage({ tableName }: { tableName: string }) {
  const detail = useService(
    (s) => lakehouseService.getTableDetail(BRONZE_NAMESPACE, tableName, s),
    [tableName]
  )
  // Maintenance is a nicety here: its failure hides one row, not the card.
  const maintenance = useService(
    (s) => lakehouseService.getMaintenance(BRONZE_NAMESPACE, tableName, s),
    [tableName]
  )

  const openTable = (
    <Button
      size="sm"
      variant="ghost"
      render={<Link href={lakehouseTableHref(BRONZE_NAMESPACE, tableName)} />}
    >
      Open table
    </Button>
  )

  if (detail.status === "loading") {
    return (
      <SectionCard size="sm" title="Storage">
        <EmptyState title="Loading table metadata…" className="py-4" />
      </SectionCard>
    )
  }
  if (detail.status === "error") {
    // A missing table is expected for a candidate the catalog guessed at.
    if (detail.error.code === "not_found") return null
    return (
      <SectionCard size="sm" title="Storage">
        <ErrorState error={detail.error} onRetry={detail.reload} />
      </SectionCard>
    )
  }

  const d = detail.data
  const { fileCount, totalBytes } = d.stats
  const avgFile =
    fileCount !== null && totalBytes !== null && fileCount > 0 ? totalBytes / fileCount : null
  const partitions = partitionLabel(d)
  const points = volumePoints(d.snapshots)
  const latest = d.snapshots.reduce<LakehouseSnapshot | null>(
    (acc, s) => (acc === null || s.timestampMs > acc.timestampMs ? s : acc),
    null
  )

  return (
    <div className="grid gap-2 lg:grid-cols-2">
      <SectionCard
        size="sm"
        title="Storage"
        description={`Iceberg table ${BRONZE_NAMESPACE}.${tableName}`}
        action={openTable}
      >
        <MetadataList
          density="compact"
          columns={2}
          items={[
            { label: "Size", value: fmtMeasured(totalBytes, formatBytes) },
            { label: "Data files", value: fmtMeasured(fileCount, formatNumber) },
            { label: "Avg file size", value: fmtMeasured(avgFile, formatBytes) },
            { label: "Snapshots", value: formatNumber(d.stats.snapshotCount) },
            {
              label: "Last load",
              value: latest ? snapshotRelativeTime(latest.timestampMs) : "—",
            },
            { label: "Table columns", value: formatNumber(d.schema.length) },
            {
              label: "Partitioned by",
              value:
                partitions.length === 0 ? (
                  "Unpartitioned"
                ) : (
                  <span className="flex flex-wrap gap-1">
                    {partitions.map((p) => (
                      <Pill key={p} tone="neutral" className="font-mono">
                        {p}
                      </Pill>
                    ))}
                  </span>
                ),
            },
            {
              label: "Maintenance",
              value:
                maintenance.status === "success"
                  ? maintenanceLabel(maintenance.data)
                  : maintenance.status === "loading"
                    ? "…"
                    : "—",
            },
          ]}
        />
      </SectionCard>

      <SectionCard size="sm" title="Row count over snapshots">
        {points.length === 0 ? (
          <EmptyState title="No snapshot reported a row count" className="py-4" />
        ) : (
          <VolumeTrend points={points} />
        )}
      </SectionCard>
    </div>
  )
}
