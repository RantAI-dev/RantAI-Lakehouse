"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useDataTable } from "@/hooks/use-data-table"
import { useService } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { overviewService } from "@/services"
import { getActivityColumns } from "./activity-columns"

/** Unified activity feed across pipelines, queries, agents, and policies. */
export function ActivityPage() {
  const state = useService((s) => overviewService.listActivity(s), [])
  const tableUrlState = useTableUrlState()
  const columns = React.useMemo(() => getActivityColumns(), [])

  const filteredData = React.useMemo(() => {
    if (state.status !== "success") return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [(r) => r.actor, (r) => r.action, (r) => r.target],
      filters: tableUrlState.filters,
      joinOperator: tableUrlState.joinOperator,
    })
  }, [
    state.status,
    state.data,
    tableUrlState.search,
    tableUrlState.filters,
    tableUrlState.joinOperator,
  ])

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/activity",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Activity"
        description="Recent actions from pipelines, queries, schema changes, policies, connectors, agents, and approvals."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="flex flex-col gap-4">
          <DataTableAdvancedToolbar table={table}>
            <DataTableSearch
              placeholder="Search actor, action, target..."
            />
          </DataTableAdvancedToolbar>
          <DataTable table={table} />
        </div>
      ) : null}
    </div>
  )
}

