"use client"

import * as React from "react"
import Link from "next/link"
import { PlayCircleIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableFacetedFilter } from "@/components/data-table/data-table-faceted-filter"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { PageHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useDataTable } from "@/hooks/use-data-table"
import { useService } from "@/hooks/use-service"
import { AGENT_RUN_STATUS_LABEL } from "@/lib/status"
import { agentService } from "@/services"
import { getRunColumns } from "./run-columns"

const STATUS_OPTIONS = Object.entries(AGENT_RUN_STATUS_LABEL).map(
  ([value, label]) => ({
    value,
    label,
  })
)

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

  const { table } = useDataTable({
    data,
    columns,
    pageCount: 1,
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (originalRow) => originalRow.id,
    shallow: false,
    clearOnDefault: true,
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
        <DataTable
          table={table}
          renderToolbar={() => (
            <DataTableAdvancedToolbar table={table}>
              <DataTableSearch
                table={table}
                placeholder="Search runs..."
                className="w-full sm:w-64"
              />
              {table.getColumn("status") && (
                <DataTableFacetedFilter
                  column={table.getColumn("status")}
                  title="Status"
                  options={STATUS_OPTIONS}
                />
              )}
            </DataTableAdvancedToolbar>
          )}
        />
      ) : null}
    </div>
  )
}
