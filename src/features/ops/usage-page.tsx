"use client"

import { PageHeader } from "@/components/patterns/page-header"
import { MetricCard, MetricGrid } from "@/components/patterns/metric-card"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { ErrorState, MetricSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill, TierBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatCompactNumber, formatPercent } from "@/lib/format"
import type { StorageTier } from "@/lib/status"
import { opsService } from "@/services"
import type { UsageSummary } from "@/services/contracts/ops"

type TenantRow = UsageSummary["tenants"][number]

function BudgetUtilization({ row }: { row: TenantRow }) {
  const fraction = row.budgetLimit > 0 ? row.budgetSpent / row.budgetLimit : 0
  if (fraction >= 0.9) {
    return (
      <div className="flex items-center gap-2">
        <Pill tone="destructive">Critical</Pill>
        <span className="text-xs font-medium text-destructive">
          {formatPercent(fraction)}
        </span>
      </div>
    )
  }
  if (fraction >= 0.75) {
    return (
      <div className="flex items-center gap-2">
        <Pill tone="warning">High</Pill>
        <span className="text-xs text-muted-foreground">
          {formatPercent(fraction)}
        </span>
      </div>
    )
  }
  return (
    <div className="flex items-center gap-2">
      <Pill tone="success">Healthy</Pill>
      <span className="text-xs text-muted-foreground">
        {formatPercent(fraction)}
      </span>
    </div>
  )
}

const columns: ColumnDef<TenantRow>[] = [
  { key: "name", header: "Tenant", render: (r) => r.name },
  {
    key: "compute",
    header: "Compute",
    render: (r) => formatCompactNumber(r.computeUnits),
  },
  {
    key: "budget",
    header: "Budget",
    render: (r) =>
      `${formatCompactNumber(r.budgetSpent)} / ${formatCompactNumber(r.budgetLimit)}`,
  },
  {
    key: "utilization",
    header: "Utilization",
    render: (r) => <BudgetUtilization row={r} />,
  },
]

export function UsagePage() {
  const state = useService((s) => opsService.getUsage(s), [])
  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Usage & Budgets"
        description="Tenant compute, storage by tier, pipeline usage, and agent budgets."
      />
      {state.status === "loading" ? <MetricSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <>
          <MetricGrid>
            <MetricCard
              label="Compute 7d"
              value={formatCompactNumber(state.data.computeUnits7d)}
            />
            <MetricCard
              label="Scanned 7d"
              value={formatBytes(state.data.scannedBytes7d)}
            />
            <MetricCard
              label="Pipeline runs 7d"
              value={formatCompactNumber(state.data.pipelineRuns7d)}
            />
            <MetricCard
              label="Agent budget used"
              value={formatPercent(state.data.agentBudgetUsedRate)}
            />
          </MetricGrid>
          <SectionCard title="Storage by tier">
            <div className="flex flex-wrap gap-3">
              {(Object.keys(state.data.storageByTier) as StorageTier[]).map(
                (t) => (
                  <div
                    key={t}
                    className="rounded-md border border-border px-3 py-2 text-sm"
                  >
                    <TierBadge tier={t} />
                    <p className="mt-1 font-semibold">
                      {formatBytes(state.data.storageByTier[t])}
                    </p>
                  </div>
                )
              )}
            </div>
          </SectionCard>
          <SectionCard title="Tenant budgets">
            <DataTable
              columns={columns}
              rows={state.data.tenants}
              rowKey={(r) => r.id}
            />
          </SectionCard>
        </>
      ) : null}
    </div>
  )
}
