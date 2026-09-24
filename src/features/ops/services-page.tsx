"use client"

import * as React from "react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService } from "@/hooks/use-service"
import { formatPercent } from "@/lib/format"
import { fmtMeasured } from "@/lib/measured"
import { opsService } from "@/services"
import type { PlatformService } from "@/services/contracts/ops"
import { getServiceColumns } from "./services-columns"

function DependencyPills({ dependencies }: { readonly dependencies: readonly string[] }) {
  if (dependencies.length === 0) return <span>—</span>
  return (
    <div className="flex flex-wrap gap-1">
      {dependencies.map((dep) => (
        <Pill key={dep} tone="neutral">
          {dep}
        </Pill>
      ))}
    </div>
  )
}

export function ServicesPage() {
  const state = useService((s) => opsService.listServices(s), [])
  const [selected, setSelected] = React.useState<PlatformService | null>(null)

  const columns = React.useMemo(
    () => getServiceColumns({ onSelect: setSelected }),
    []
  )

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(state.data ?? [], {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.name,
          (r) => r.site,
          (r) => r.version,
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
    persistKey: "/services",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Services"
        description="Platform service health, versions, sites, and dependencies."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch
              placeholder="Search name, site..."
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
        description={selected ? `Platform service on ${selected.site}` : undefined}
      >
        {selected ? (
          <>
            <div className="flex items-center gap-1 self-start">
              <HealthBadge health={selected.health} />
              {!selected.checked ? (
                <span className="text-xs text-muted-foreground">Not probed</span>
              ) : null}
            </div>
            <MetadataList
              items={[
                {
                  label: "Version",
                  value: (
                    <span className="font-mono text-xs">
                      {selected.version === null ? "—" : `v${selected.version}`}
                    </span>
                  ),
                },
                { label: "Site", value: selected.site },
                { label: "Replicas", value: fmtMeasured(selected.replicas) },
                {
                  label: "Error rate",
                  value: fmtMeasured(selected.errorRate, formatPercent),
                },
                {
                  label: "Latency",
                  value: (
                    <span className="font-mono text-xs">
                      {fmtMeasured(selected.latencyMs, (n) => `${n} ms`)}
                    </span>
                  ),
                },
                {
                  label: "Dependencies",
                  value: <DependencyPills dependencies={selected.dependencies} />,
                },
              ]}
            />
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}
