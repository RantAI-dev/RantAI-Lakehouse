"use client"

import Link from "next/link"
import { BellRing, RotateCcw } from "lucide-react"

import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { MetricCard, MetricGrid } from "@/components/patterns/metric-card"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
  MetricSkeleton,
} from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useService, type ServiceState } from "@/hooks/use-service"
import {
  formatBytes,
  formatCompactNumber,
  formatDateTime,
  formatPercent,
  formatRelativeTime,
} from "@/lib/format"
import { formatSignedBytes } from "@/lib/lakehouse-view"
import type { Measured } from "@/lib/measured"
import {
  latestRunPerTable,
  maintenanceTableHref,
  overallServiceState,
} from "@/lib/system-status"
import { cn } from "@/lib/utils"
import { governanceService, lakehouseService, overviewService } from "@/services"
import type { MaintenanceRun } from "@/services/contracts/governance"
import type { LakehouseCapacityBucket } from "@/services/contracts/lakehouse"
import type { PlatformService } from "@/services/contracts/ops"

type Reloadable<T> = ServiceState<T> & { reload: () => void }

/**
 * A value the API did not measure (S2-D2: "Not measured", never a blank or
 * a made-up number). Muted, so it reads as an absence, not as a reading.
 */
function NotMeasured() {
  return <span className="text-xs text-muted-foreground">Not measured</span>
}

function measured(value: Measured, format: (n: number) => string = String) {
  return value === null ? <NotMeasured /> : <span className="font-mono text-xs">{format(value)}</span>
}

const OVERALL_LABEL = {
  success: "Up",
  destructive: "Not healthy",
  warning: "Partly probed",
  neutral: "Unknown",
} as const

/**
 * Section 1: one line that answers "is it up?", and the open alerts.
 *
 * The services state is the page's, shared with the Services table, so the
 * line and the table cannot disagree. The alerts load on their own; if they
 * fail, only the alerts half says so.
 */
export function StatusSummary({
  services,
  refreshKey,
}: {
  services: Reloadable<PlatformService[]>
  refreshKey: number
}) {
  const alerts = useService((s) => overviewService.listAlerts(s), [refreshKey])

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-border bg-card p-4 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)] sm:flex-row sm:items-center sm:justify-between">
      <div className="flex min-w-0 flex-col gap-1">
        {services.status === "loading" ? (
          <Skeleton className="h-6 w-64" role="status" aria-label="Loading service state" />
        ) : services.status === "error" ? (
          <p className="text-sm text-muted-foreground">
            Whether the services are up could not be read. See Services below.
          </p>
        ) : (
          <OverallLine services={services.data} />
        )}
      </div>
      <div className="shrink-0">
        {alerts.status === "loading" ? (
          <Skeleton className="h-8 w-32" role="status" aria-label="Loading alerts" />
        ) : alerts.status === "error" ? (
          <span className="flex items-center gap-1 text-xs text-muted-foreground">
            Alerts could not load.
            <Button size="sm" variant="ghost" onClick={alerts.reload}>
              <RotateCcw className="size-3.5" aria-hidden />
              Retry
            </Button>
          </span>
        ) : (
          <OpenAlertsLink count={alerts.data.filter((a) => a.status === "open").length} />
        )}
      </div>
    </div>
  )
}

function OverallLine({ services }: { services: PlatformService[] }) {
  const state = overallServiceState(services)
  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <Pill tone={state.tone}>
          <span className="size-1.5 rounded-full bg-current" aria-hidden />
          {OVERALL_LABEL[state.tone]}
        </Pill>
        <p className="text-sm font-medium text-foreground">{state.headline}</p>
      </div>
      {state.notProbed.length > 0 ? (
        <p className="text-xs text-muted-foreground">
          Not probed, so not counted: {state.notProbed.join(", ")}.
        </p>
      ) : null}
    </>
  )
}

function OpenAlertsLink({ count }: { count: number }) {
  return (
    <Button
      size="sm"
      variant="outline"
      className={cn(count > 0 && "border-destructive/40 text-destructive")}
      render={<Link href="/alerts" />}
    >
      <BellRing className="size-4" aria-hidden />
      {count === 0 ? "No open alerts" : count === 1 ? "1 open alert" : `${count} open alerts`}
    </Button>
  )
}

const serviceColumns: ColumnDef<PlatformService>[] = [
  {
    key: "name",
    header: "Service",
    render: (s) => (
      <div className="min-w-0">
        <p className="font-medium">{s.name}</p>
        <p className="text-xs text-muted-foreground">{s.site}</p>
      </div>
    ),
    sortValue: (s) => s.name,
  },
  {
    key: "health",
    header: "Health",
    render: (s) => (
      <div className="flex flex-wrap items-center gap-1">
        <HealthBadge health={s.health} />
        {!s.checked ? <span className="text-xs text-muted-foreground">Not probed</span> : null}
      </div>
    ),
    sortValue: (s) => s.health,
  },
  {
    key: "version",
    header: "Version",
    render: (s) =>
      s.version === null ? <NotMeasured /> : <span className="font-mono text-xs">v{s.version}</span>,
  },
  {
    key: "latencyMs",
    header: "Latency",
    render: (s) => measured(s.latencyMs, (n) => `${n} ms`),
  },
  {
    key: "errorRate",
    header: "Error rate",
    render: (s) => measured(s.errorRate, formatPercent),
  },
  {
    key: "replicas",
    header: "Replicas",
    render: (s) => measured(s.replicas),
  },
  {
    key: "dependencies",
    header: "Depends on",
    render: (s) =>
      s.dependencies.length === 0 ? (
        <span className="text-xs text-muted-foreground">None</span>
      ) : (
        <div className="flex flex-wrap gap-1">
          {s.dependencies.map((dep) => (
            <Pill key={dep} tone="neutral">
              {dep}
            </Pill>
          ))}
        </div>
      ),
  },
]

/** Section 2: the probe table, as `GET /api/ops/services` returns it. */
export function ServicesSection({ services }: { services: Reloadable<PlatformService[]> }) {
  return (
    <SectionCard
      title="Services"
      description="What the API probed for this request. A service it does not probe says so, and a value nothing measures reads Not measured."
    >
      {services.status === "loading" ? <LoadingSkeleton rows={4} /> : null}
      {services.status === "error" ? (
        <ErrorState error={services.error} onRetry={services.reload} className="py-6" />
      ) : null}
      {services.status === "success" ? (
        <DataTable
          columns={serviceColumns}
          rows={services.data}
          rowKey={(s) => s.id}
          emptyMessage="The API listed no services."
        />
      ) : null}
    </SectionCard>
  )
}

const bucketColumns: ColumnDef<LakehouseCapacityBucket>[] = [
  {
    key: "name",
    header: "Bucket",
    render: (b) => <span className="font-mono text-sm">{b.name}</span>,
    sortValue: (b) => b.name,
  },
  {
    key: "bytes",
    header: "Size",
    render: (b) => formatBytes(b.bytes),
    sortValue: (b) => b.bytes,
  },
  {
    key: "objects",
    header: "Objects",
    render: (b) => b.objects.toLocaleString("en"),
    sortValue: (b) => b.objects,
  },
  {
    key: "measuredAt",
    header: "Measured",
    render: (b) => (
      <span className="text-xs text-muted-foreground" title={formatDateTime(b.measuredAt)}>
        {formatRelativeTime(b.measuredAt)}
      </span>
    ),
  },
]

/**
 * Section 3: storage, from `GET /api/lakehouse/capacity` (the daily
 * `capacity_snapshot_job` per bucket, plus `ClickHouse`'s live disk).
 * `growth7d` is `null`, not zero, until a reading falls inside the
 * seven-day window, and reads "Not measured" then.
 */
export function StorageSection({ refreshKey }: { refreshKey: number }) {
  const capacity = useService((s) => lakehouseService.getCapacity(s), [refreshKey])

  return (
    <SectionCard
      title="Storage"
      description="ClickHouse's disk right now, and each bucket as the daily capacity snapshot last measured it."
    >
      {capacity.status === "loading" ? (
        <div className="flex flex-col gap-3">
          <MetricSkeleton cards={2} className="lg:grid-cols-2" />
          <LoadingSkeleton rows={2} />
        </div>
      ) : null}
      {capacity.status === "error" ? (
        <ErrorState error={capacity.error} onRetry={capacity.reload} className="py-6" />
      ) : null}
      {capacity.status === "success" ? (
        <div className="flex flex-col gap-4">
          <MetricGrid className="sm:grid-cols-2 lg:grid-cols-2">
            <MetricCard
              label="ClickHouse on disk"
              value={formatBytes(capacity.data.clickhouse.bytesOnDisk)}
            />
            <MetricCard
              label="Growth, last 7 days"
              value={
                capacity.data.growth7d === null
                  ? "Not measured"
                  : formatSignedBytes(capacity.data.growth7d)
              }
              hint={
                capacity.data.growth7d === null
                  ? "Needs a daily reading from at least 7 days ago."
                  : undefined
              }
            />
          </MetricGrid>
          {capacity.data.buckets.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No bucket reading yet: the capacity snapshot runs once a day.
            </p>
          ) : (
            <DataTable columns={bucketColumns} rows={capacity.data.buckets} rowKey={(b) => b.name} />
          )}
        </div>
      ) : null}
    </SectionCard>
  )
}

const runColumns: ColumnDef<MaintenanceRun>[] = [
  {
    key: "tableName",
    header: "Table",
    render: (r) => {
      const href = maintenanceTableHref(r.tableName)
      return href ? (
        <Link href={href} className="font-mono text-sm text-primary hover:underline">
          {r.tableName}
        </Link>
      ) : (
        <span className="font-mono text-sm">{r.tableName}</span>
      )
    },
    sortValue: (r) => r.tableName,
  },
  {
    key: "runAt",
    header: "Last run",
    render: (r) => (
      <span className="text-xs text-muted-foreground" title={formatDateTime(r.runAt)}>
        {formatRelativeTime(r.runAt)}
      </span>
    ),
    sortValue: (r) => r.runAt,
  },
  {
    key: "dryRun",
    header: "Dry run would delete",
    render: (r) => (
      <span className="font-mono text-xs text-muted-foreground">
        {r.dryRun.deletedDataFiles} data / {r.dryRun.deletedManifestFiles} manifest
      </span>
    ),
  },
  {
    key: "applied",
    header: "Deleted",
    render: (r) => (
      <span className="font-mono text-xs">
        {r.applied.deletedDataFiles} data / {r.applied.deletedManifestFiles} manifest
      </span>
    ),
  },
  {
    key: "skippedVerbs",
    header: "Skipped",
    render: (r) =>
      r.skippedVerbs ? (
        <Pill tone="warning" title={r.skippedVerbs}>
          {r.skippedVerbs}
        </Pill>
      ) : (
        <span className="text-xs text-muted-foreground">None</span>
      ),
  },
]

/**
 * Section 5: what the nightly maintenance job last did to each table
 * (`GET /api/governance/maintenance`, `bronze_maintenance_job`). On
 * ClickHouse 26.8 the working verb is `remove_orphan_files`;
 * `expire_snapshots` is refused behind a REST catalog and recorded as
 * skipped; compaction runs through Trino (ADR 0009) and has no record here
 * (`docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`). Each row links to its
 * table's page, where its maintenance policy is set.
 */
export function MaintenanceSection({ refreshKey }: { refreshKey: number }) {
  const runs = useService((s) => governanceService.listMaintenanceRuns(s), [refreshKey])

  return (
    <SectionCard
      title="Table maintenance"
      description="The latest nightly run on each table: orphan files found (dry run) and removed. Snapshot expiry is refused on this ClickHouse version and shows as skipped; compaction runs through Trino and is not recorded here. Open a table to set its policy."
    >
      {runs.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
      {runs.status === "error" ? (
        <ErrorState error={runs.error} onRetry={runs.reload} className="py-6" />
      ) : null}
      {runs.status === "success" ? (
        runs.data.length === 0 ? (
          <EmptyState
            title="No maintenance run recorded yet"
            description="The maintenance job runs nightly over the Bronze tables."
            className="py-6"
          />
        ) : (
          <DataTable
            columns={runColumns}
            rows={latestRunPerTable(runs.data)}
            rowKey={(r) => r.tableName}
            pageSize={10}
          />
        )
      ) : null}
    </SectionCard>
  )
}

/**
 * Section 6, first half: the two query cards Health had, from the
 * overview summary. Only these two: the rest of that summary is shown on
 * the pages it belongs to (S2-D2 lists what this page shows).
 */
export function QueriesSection({ refreshKey }: { refreshKey: number }) {
  const summary = useService((s) => overviewService.getSummary(s), [refreshKey])

  return (
    <SectionCard
      title="Queries, last 24 hours"
      action={
        <Link href="/query-studio/history" className="text-sm text-primary hover:underline">
          Query history
        </Link>
      }
    >
      {summary.status === "loading" ? (
        <MetricSkeleton cards={2} className="lg:grid-cols-2" />
      ) : null}
      {summary.status === "error" ? (
        <ErrorState error={summary.error} onRetry={summary.reload} className="py-6" />
      ) : null}
      {summary.status === "success" ? (
        <MetricGrid className="sm:grid-cols-2 lg:grid-cols-2">
          <MetricCard
            label="Query volume"
            value={formatCompactNumber(summary.data.queries.volume24h)}
            hint={`p95 ${summary.data.queries.p95Ms} ms · cache ${
              summary.data.queries.cacheAssistRate === null
                ? "not measured"
                : formatPercent(summary.data.queries.cacheAssistRate)
            }`}
            href="/query-studio/history"
          />
          <MetricCard
            label="Query failure rate"
            value={formatPercent(summary.data.queries.failureRate)}
            trendTone={summary.data.queries.failureRate > 0 ? "negative" : "positive"}
            href="/query-studio/history"
          />
        </MetricGrid>
      ) : null}
    </SectionCard>
  )
}
