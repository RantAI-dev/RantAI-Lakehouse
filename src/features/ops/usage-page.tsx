"use client"

import * as React from "react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { MetricCard, MetricGrid } from "@/components/patterns/metric-card"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, MetricSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { TierBadge } from "@/components/patterns/status-badge"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatCompactNumber, formatPercent } from "@/lib/format"
import type { StorageTier } from "@/lib/status"
import { opsService } from "@/services"
import type { UsageSummary } from "@/services/contracts/ops"
import { getUsageTenantColumns } from "./usage-columns"

type TenantRow = UsageSummary["tenants"][number]

function TenantBudgetsTable({ tenants }: { readonly tenants: readonly TenantRow[] }) {
  const columns = React.useMemo(() => getUsageTenantColumns(), [])

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(tenants as TenantRow[], {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.name,
        ],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [tenants, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator]
  )

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/usage",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  return (
    <div className="space-y-4">
      <DataTableAdvancedToolbar table={table}>
        <DataTableSearch
          placeholder="Search tenants..."
        />
      </DataTableAdvancedToolbar>
      <div className="rounded-md border">
        <DataTable table={table} />
      </div>
    </div>
  )
}

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
            <TenantBudgetsTable tenants={state.data.tenants} />
          </SectionCard>
        </>
      ) : null}
    </div>
  )
}
