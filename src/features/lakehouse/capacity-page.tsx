"use client"

import type { ColumnDef } from "@/components/patterns/data-table"
import { DataTable } from "@/components/patterns/data-table"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatDateTime } from "@/lib/format"
import { formatSignedBytes } from "@/lib/lakehouse-view"
import { fmtMeasured } from "@/lib/measured"
import { lakehouseService } from "@/services"
import type { LakehouseCapacityBucket } from "@/services/contracts/lakehouse"

const bucketColumns: ColumnDef<LakehouseCapacityBucket>[] = [
  { key: "name", header: "Bucket", render: (b) => <span className="font-mono text-sm">{b.name}</span> },
  { key: "bytes", header: "Size", render: (b) => formatBytes(b.bytes) },
  { key: "objects", header: "Objects", render: (b) => b.objects.toLocaleString("en") },
  { key: "measuredAt", header: "Measured at", render: (b) => formatDateTime(b.measuredAt) },
]

/**
 * The Lakehouse capacity surface: the daily `capacity_snapshot_job`'s
 * per-bucket readings plus `ClickHouse`'s own live disk usage (WS2 §4).
 *
 * `growth7d` renders through `fmtMeasured` rather than `?? 0` — it is
 * `null`, not zero, whenever no reading falls within the seven-day growth
 * window (fewer than ~8 days of history yet, or a missed daily run), and
 * showing `0` there would misreport genuine no-growth.
 */
export function LakehouseCapacityPage() {
  const capacityState = useService((s) => lakehouseService.getCapacity(s), [])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Lakehouse capacity"
        description="Per-bucket storage from the daily capacity snapshot, plus ClickHouse's live disk usage."
      />

      {capacityState.status === "loading" ? (
        <SectionCard size="sm" title="Capacity">
          <LoadingSkeleton rows={3} />
        </SectionCard>
      ) : null}

      {capacityState.status === "error" ? (
        <SectionCard size="sm" title="Capacity">
          <ErrorState error={capacityState.error} onRetry={capacityState.reload} />
        </SectionCard>
      ) : null}

      {capacityState.status === "success" ? (
        <>
          <SectionCard size="sm" title="ClickHouse disk usage">
            <div className="flex flex-wrap gap-8">
              <div>
                <div className="text-xs text-muted-foreground">Bytes on disk</div>
                <div className="text-lg font-semibold">
                  {formatBytes(capacityState.data.clickhouse.bytesOnDisk)}
                </div>
              </div>
              <div>
                <div className="text-xs text-muted-foreground">7-day growth</div>
                <div className="text-lg font-semibold">
                  {fmtMeasured(capacityState.data.growth7d, formatSignedBytes)}
                </div>
              </div>
            </div>
          </SectionCard>

          <SectionCard size="sm" title="Buckets">
            {capacityState.data.buckets.length === 0 ? (
              <EmptyState title="No capacity readings yet" className="py-4" />
            ) : (
              <DataTable
                columns={bucketColumns}
                rows={capacityState.data.buckets}
                rowKey={(b) => b.name}
              />
            )}
          </SectionCard>
        </>
      ) : null}
    </div>
  )
}
