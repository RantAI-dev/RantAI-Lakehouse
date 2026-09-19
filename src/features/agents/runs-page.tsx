"use client"

import * as React from "react"
import Link from "next/link"
import { PlayCircleIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { PageHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService } from "@/hooks/use-service"
import { agentService } from "@/services"
import { getRunColumns } from "./run-columns"

/**
 * `/agents/runs` — every digital-employee run, real ones only: seeded
 * fixtures were dropped (migration `0025_drop_seeded_agent_runs.sql`), so
 * this list is legitimately empty until a Run now click or a schedule
 * produces the first row. The empty state says so explicitly rather than
 * reading as broken.
 */
export function RunsPage() {
  const state = useService((s) => agentService.listRuns(undefined, s), [])

  const columns = React.useMemo(() => getRunColumns(), [])

  const data = state.data ?? []

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(state.data ?? [], {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.id,
          (r) => r.employeeId,
          (r) => r.trigger,
          (r) => r.actor,
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
    persistKey: "/agents/runs",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Agent Runs"
        description="Every headless execution of a digital employee — triggered manually with Run now, or by a schedule."
        actions={
          <Button size="sm" variant="outline" render={<Link href="/agents/employees" />}>
            <PlayCircleIcon data-icon="inline-start" />
            Run an employee
          </Button>
        }
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" && data.length === 0 ? (
        <EmptyState
          title="No runs yet"
          description="Runs appear here once a digital employee actually executes — click Run now on an employee's page, or set a schedule so it runs on its own. There's nothing broken; the seeded demo history was intentionally removed."
          action={
            <Button size="sm" render={<Link href="/agents/employees" />}>
              Go to Digital Employees
            </Button>
          }
        />
      ) : null}
      {state.status === "success" && data.length > 0 ? (
        <DataTable table={table}>
          <DataTableAdvancedToolbar table={table}>
            <DataTableSearch placeholder="Search runs..." />
          </DataTableAdvancedToolbar>
        </DataTable>
      ) : null}
    </div>
  )
}
