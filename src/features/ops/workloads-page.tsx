"use client"

import * as React from "react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService, useServiceAction } from "@/hooks/use-service"
import { opsService } from "@/services"
import { getWorkloadColumns } from "./workloads-columns"

export function WorkloadsPage() {
  const state = useService((s) => opsService.listWorkloads(s), [])
  const [cancellingId, setCancellingId] = React.useState<string | null>(null)

  const cancelAction = useServiceAction((signal, id: string) =>
    opsService.cancelWorkload(id, signal)
  )

  const handleCancel = React.useCallback(
    async (id: string) => {
      setCancellingId(id)
      const result = await cancelAction.run(id)
      setCancellingId(null)
      if (result) state.reload()
    },
    [cancelAction, state]
  )

  const columns = React.useMemo(
    () =>
      getWorkloadColumns({
        onCancel: handleCancel,
        cancellingId,
        isCancelPending: cancelAction.status === "pending",
      }),
    [handleCancel, cancellingId, cancelAction.status]
  )

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(state.data ?? [], {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.principal,
          (r) => r.tenant,
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
    persistKey: "/workloads",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Workloads"
        description="Active and queued requests with class, engine category, cost, and fairness signals."
      />
      {cancelAction.status === "error" ? (
        <p className="text-xs text-destructive" role="alert">
          Failed to cancel workload: {cancelAction.error.message}
        </p>
      ) : null}
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch
              placeholder="Search principal, tenant..."
            />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}
    </div>
  )
}
