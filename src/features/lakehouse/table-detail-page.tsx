"use client"

import { useParams } from "next/navigation"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useService } from "@/hooks/use-service"
import { formatBytes } from "@/lib/format"
import {
  maintenanceSummary,
  snapshotRelativeTime,
  snapshotsNewestFirst,
} from "@/lib/lakehouse-view"
import { fmtMeasured } from "@/lib/measured"
import { lakehouseService } from "@/services"

/**
 * Read-only Iceberg table detail: schema, partition spec, a newest-first
 * snapshot timeline, stats, and the current maintenance policy (WS2 §4).
 * The write form for the maintenance policy is a later task (B4).
 */
export function LakehouseTableDetailPage() {
  const params = useParams<{ namespace: string; table: string }>()
  const detailState = useService(
    (s) => lakehouseService.getTableDetail(params.namespace, params.table, s),
    [params.namespace, params.table]
  )
  const maintenanceState = useService(
    (s) => lakehouseService.getMaintenance(params.namespace, params.table, s),
    [params.namespace, params.table]
  )

  if (detailState.status === "loading") return <LoadingSkeleton rows={8} />
  if (detailState.status === "error") {
    if (detailState.error.code === "not_found") {
      return (
        <EmptyState
          title="Table not found"
          description={`${params.namespace}.${params.table} does not exist in this warehouse.`}
        />
      )
    }
    return <ErrorState error={detailState.error} onRetry={detailState.reload} />
  }
  const detail = detailState.data
  const snapshots = snapshotsNewestFirst(detail.snapshots)

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title={`${params.namespace}.${params.table}`}
        description="Iceberg schema, partitioning, snapshots, and maintenance policy."
      />

      <SectionCard size="sm" title="Schema">
        {detail.schema.length === 0 ? (
          <EmptyState title="No schema fields" className="py-4" />
        ) : (
          <div className="overflow-hidden rounded-lg border border-border">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead>ID</TableHead>
                  <TableHead>Name</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Required</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {detail.schema.map((f) => (
                  <TableRow key={f.id}>
                    <TableCell className="font-mono text-xs">{f.id}</TableCell>
                    <TableCell className="font-mono text-xs">{f.name}</TableCell>
                    <TableCell className="font-mono text-xs">{f.type}</TableCell>
                    <TableCell>{f.required ? "Yes" : "No"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </SectionCard>

      <SectionCard size="sm" title="Partition spec">
        {detail.partitionSpec.length === 0 ? (
          <EmptyState title="This table is unpartitioned" className="py-4" />
        ) : (
          <ul className="divide-y divide-border text-sm">
            {detail.partitionSpec.map((p) => (
              <li
                key={`${p.sourceId}-${p.name}`}
                className="flex flex-wrap items-baseline gap-2 py-1.5"
              >
                <span className="font-mono font-medium">{p.name}</span>
                <span className="text-muted-foreground">{p.transform}</span>
                <span className="ml-auto text-xs text-muted-foreground">
                  source #{p.sourceId}
                </span>
              </li>
            ))}
          </ul>
        )}
      </SectionCard>

      <SectionCard size="sm" title="Snapshots" description="Newest first.">
        {snapshots.length === 0 ? (
          <EmptyState title="No snapshots for this table" className="py-4" />
        ) : (
          <ul className="divide-y divide-border text-sm">
            {snapshots.map((snap) => (
              <li key={snap.id} className="flex justify-between gap-2 py-1.5">
                <span>{snap.operation}</span>
                <span className="text-muted-foreground">
                  {snapshotRelativeTime(snap.timestampMs)} ·{" "}
                  {fmtMeasured(snap.summary.totalRecords)} records
                </span>
              </li>
            ))}
          </ul>
        )}
      </SectionCard>

      <SectionCard size="sm" title="Stats">
        <dl className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-3">
          <div>
            <dt className="text-xs text-muted-foreground">Files</dt>
            <dd>{fmtMeasured(detail.stats.fileCount)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Small files</dt>
            <dd>{fmtMeasured(detail.stats.smallFileCount)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Records</dt>
            <dd>{fmtMeasured(detail.stats.recordCount)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Size</dt>
            <dd>{fmtMeasured(detail.stats.totalBytes, formatBytes)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Snapshots</dt>
            <dd>{detail.stats.snapshotCount}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Metadata log entries</dt>
            <dd>{detail.stats.metadataLogCount}</dd>
          </div>
        </dl>
      </SectionCard>

      <SectionCard
        size="sm"
        title="Maintenance policy"
        description="Read-only. Configuring a policy is not available yet."
      >
        {maintenanceState.status === "loading" ? <LoadingSkeleton rows={2} /> : null}
        {maintenanceState.status === "error" ? (
          <ErrorState error={maintenanceState.error} onRetry={maintenanceState.reload} />
        ) : null}
        {maintenanceState.status === "success" ? (
          <div className="flex flex-col gap-2 text-sm">
            <p>{maintenanceSummary(maintenanceState.data)}</p>
            <p className="text-muted-foreground">
              {maintenanceState.data.lastRun === null
                ? "No maintenance run recorded"
                : `Last run ${maintenanceState.data.lastRun.runAt} — applied ${maintenanceState.data.lastRun.applied.deletedDataFiles} data file(s), ${maintenanceState.data.lastRun.applied.deletedManifestFiles} manifest file(s) deleted`}
            </p>
          </div>
        ) : null}
      </SectionCard>
    </div>
  )
}
