"use client"

import { PageHeader } from "@/components/patterns/page-header"
import { MetricCard, MetricGrid } from "@/components/patterns/metric-card"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  ErrorState,
  LoadingSkeleton,
  MetricSkeleton,
} from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { StatusBadge, TierBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatPercent, formatRelativeTime } from "@/lib/format"
import { STORAGE_TIER_LABEL, type StorageTier } from "@/lib/status"
import { storageService } from "@/services"
import type { LifecyclePolicy, TieringOp } from "@/services/contracts/storage"

const TIERS: StorageTier[] = ["hot", "warm", "cold", "ai"]

const policyCols: ColumnDef<LifecyclePolicy>[] = [
  { key: "name", header: "Policy", render: (r) => r.name },
  { key: "scope", header: "Scope", render: (r) => r.scope },
  {
    key: "rules",
    header: "Hot → Warm → Cold",
    render: (r) => `${r.hotDays}d → ${r.warmDays}d → ${r.coldAfterDays}d+`,
  },
  {
    key: "status",
    header: "Status",
    render: (r) => <StatusBadge status={r.status} />,
  },
  { key: "savings", header: "Savings", render: (r) => r.estimatedSavings },
  {
    key: "applied",
    header: "Last applied",
    render: (r) => (
      <span className="text-muted-foreground">
        {formatRelativeTime(r.lastAppliedAt)}
      </span>
    ),
  },
]

const opCols: ColumnDef<TieringOp>[] = [
  { key: "asset", header: "Asset", render: (r) => r.asset },
  {
    key: "move",
    header: "Move",
    render: (r) => (
      <span className="inline-flex items-center gap-1">
        <TierBadge tier={r.from} />
        <span className="text-muted-foreground">→</span>
        <TierBadge tier={r.to} />
      </span>
    ),
  },
  {
    key: "status",
    header: "Status",
    render: (r) => <StatusBadge status={r.status} />,
  },
  { key: "at", header: "When", render: (r) => formatRelativeTime(r.at) },
  { key: "detail", header: "Detail", render: (r) => r.detail },
]

export function StoragePage() {
  const overview = useService((s) => storageService.getOverview(s), [])
  const policies = useService((s) => storageService.listPolicies(s), [])
  const ops = useService((s) => storageService.listOperations(s), [])
  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Storage Lifecycle"
        description="Hot → Warm → Cold tiering with AI derivative datasets, savings, and restore operations."
      />
      {overview.status === "loading" ? <MetricSkeleton /> : null}
      {overview.status === "error" ? (
        <ErrorState error={overview.error} onRetry={overview.reload} />
      ) : null}
      {overview.status === "success" ? (
        <>
          <MetricGrid>
            {TIERS.map((t) => (
              <MetricCard
                key={t}
                label={STORAGE_TIER_LABEL[t]}
                value={formatBytes(overview.data.byTier[t].bytes)}
                hint={`${overview.data.byTier[t].assets} assets · ${formatPercent(overview.data.byTier[t].growth7d)} 7d growth`}
              />
            ))}
          </MetricGrid>
          <MetricGrid className="lg:grid-cols-3">
            <MetricCard
              label="Savings vs all-hot"
              value={formatPercent(overview.data.savingsVsAllHot)}
            />
            <MetricCard
              label="Failed tiering ops"
              value={overview.data.failedTieringOps}
              trendTone="negative"
            />
            <MetricCard label="Pending restores" value={overview.data.pendingRestores} />
          </MetricGrid>
          <SectionCard
            title="Tier lane"
            description="Physical lifecycle path for analytical data."
          >
            <div className="flex flex-wrap items-center gap-2">
              <TierBadge tier="hot" />
              <span className="text-muted-foreground" aria-hidden>
                →
              </span>
              <TierBadge tier="warm" />
              <span className="text-muted-foreground" aria-hidden>
                →
              </span>
              <TierBadge tier="cold" />
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <span className="text-muted-foreground" aria-hidden>
                →
              </span>
              <TierBadge tier="ai" />
              <span className="text-xs text-muted-foreground">
                derivative datasets rebuilt from lineage
              </span>
            </div>
          </SectionCard>
        </>
      ) : null}

      <SectionCard title="Lifecycle policies">
        {policies.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
        {policies.status === "error" ? (
          <ErrorState error={policies.error} onRetry={policies.reload} />
        ) : null}
        {policies.status === "success" ? (
          <DataTable columns={policyCols} rows={policies.data} rowKey={(r) => r.id} />
        ) : null}
      </SectionCard>

      <SectionCard title="Recent tiering operations">
        {ops.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
        {ops.status === "error" ? (
          <ErrorState error={ops.error} onRetry={ops.reload} />
        ) : null}
        {ops.status === "success" ? (
          <DataTable columns={opCols} rows={ops.data} rowKey={(r) => r.id} />
        ) : null}
      </SectionCard>
    </div>
  )
}
