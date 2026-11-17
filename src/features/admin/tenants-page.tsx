"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useAuth } from "@/features/auth/auth-provider"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService } from "@/hooks/use-service"
import {
  formatBytes,
  formatCompactNumber,
  formatNumber,
  formatPercent,
} from "@/lib/format"
import { identityService } from "@/services"
import type { Tenant } from "@/services/contracts/identity"
import { getTenantColumns } from "./tenants-columns"

export function TenantsPage() {
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("identity:write")
  const state = useService((s) => identityService.listTenants(s), [])
  const [selected, setSelected] = React.useState<Tenant | null>(null)

  const columns = React.useMemo(
    () => getTenantColumns({ onSelect: setSelected }),
    []
  )

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(state.data ?? [], {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.name,
          (r) => r.slug,
          (r) => r.plan,
        ],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [state.data, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator]
  )

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/admin/tenants",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Tenants"
        description="Tenant identity, residency, quotas, agents, and storage posture."
        actions={
          <Button
            size="sm"
            render={<Link href="/admin/tenants/new" />}
            disabled={!canWrite}
            title={canWrite ? undefined : "You don't have permission to create tenants."}
          >
            <PlusIcon data-icon="inline-start" />
            Create Tenant
          </Button>
        }
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch
              placeholder="Search name, slug, plan..."
            />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}
      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description={selected?.slug}
      >
        {selected ? (
          <MetadataList
            items={[
              { label: "Plan", value: selected.plan },
              { label: "Residency", value: selected.residency },
              { label: "Users", value: formatNumber(selected.users) },
              { label: "Agents", value: formatNumber(selected.agents) },
              { label: "Storage", value: formatBytes(selected.storageBytes) },
              {
                label: "Compute used",
                value: formatCompactNumber(selected.usedCompute),
              },
              {
                label: "Compute quota",
                value: formatCompactNumber(selected.quotaCompute),
              },
              {
                label: "Utilization",
                value:
                  selected.quotaCompute > 0
                    ? formatPercent(selected.usedCompute / selected.quotaCompute)
                    : "—",
              },
            ]}
          />
        ) : null}
      </DetailDrawer>
    </div>
  )
}
