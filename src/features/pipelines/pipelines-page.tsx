"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { PlusIcon, RefreshCw, SparklesIcon } from "lucide-react"

import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DataTableSkeleton } from "@/components/data-table/data-table-skeleton"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { withNotify } from "@/lib/notify"
import { cn } from "@/lib/utils"
import {
  activeFilterValues,
  removeFilter,
  toggleFilterValue,
} from "@/lib/table-filter-link"
import { useSearchParams } from "next/navigation"
import { pipelineService } from "@/services"
import { AgenticBuilderDialog } from "./agentic-builder-dialog"
import { getPipelineColumns } from "./pipeline-columns"

/** Statuses worth a one-click filter on an operational list. */
const QUICK_STATUSES = [
  { value: "running", label: "Running" },
  { value: "failed", label: "Failed" },
  { value: "paused", label: "Paused" },
  { value: "scheduled", label: "Scheduled" },
]

export function PipelinesPage() {
  const router = useRouter()
  const searchParams = useSearchParams()
  const state = useService((s) => pipelineService.listPipelines(s), [])
  const [agenticOpen, setAgenticOpen] = React.useState(false)
  const tableUrlState = useTableUrlState()

  const triggerAction = useServiceAction(
    withNotify(
      { success: "Run triggered", error: "Could not trigger a run" },
      (signal, id: string) => pipelineService.triggerRun(id, signal)
    )
  )

  const columns = React.useMemo(
    () =>
      getPipelineColumns({
        onView: (p) => router.push(`/pipelines/${p.id}`),
        // Only an orchestrator job can be launched; an authored pipeline
        // has no engine behind it, and the menu says so rather than
        // offering an action that always fails.
        onTrigger: async (p) => {
          const run = await triggerAction.run(p.id)
          if (run) state.reload()
        },
      }),
    [router, triggerAction, state]
  )

  const pipelines = React.useMemo(
    () => (state.status === "success" ? state.data.pipelines : []),
    [state]
  )

  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(pipelines, {
        search: tableUrlState.search,
        searchFields: [
          (p) => p.name,
          (p) => p.id,
          (p) => p.source,
          (p) => p.target,
          (p) => p.owner,
          (p) => p.kind,
        ],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [
      pipelines,
      tableUrlState.search,
      tableUrlState.filters,
      tableUrlState.joinOperator,
    ]
  )

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/pipelines",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  // The status chips own the `status` filter and carry the rest of the URL
  // through untouched.
  const activeStatuses = React.useMemo(
    () => activeFilterValues(tableUrlState.filters, "status"),
    [tableUrlState.filters]
  )
  const statusHref = React.useCallback(
    (filters: string) => {
      const next = new URLSearchParams(searchParams.toString())
      if (filters) next.set("filters", filters)
      else next.delete("filters")
      const query = next.toString()
      return query ? `/pipelines?${query}` : "/pipelines"
    },
    [searchParams]
  )

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Pipelines"
        description="Batch and incremental flows with run health and freshness."
        actions={
          <>
            <Button
              variant="outline"
              size="sm"
              onClick={state.reload}
              disabled={state.status === "loading"}
            >
              <RefreshCw
                className={cn("size-4", state.status === "loading" && "animate-spin")}
              />
              Refresh
            </Button>
            <Button variant="outline" size="sm" onClick={() => setAgenticOpen(true)}>
              <SparklesIcon data-icon="inline-start" />
              Agentic Builder
            </Button>
            <Button size="sm" render={<Link href="/pipelines/create" />}>
              <PlusIcon data-icon="inline-start" />
              Create Pipeline
            </Button>
          </>
        }
      />

      {state.status === "loading" ? (
        <DataTableSkeleton columnCount={7} rowCount={6} filterCount={2} />
      ) : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}

      {state.status === "success" && state.data.error ? (
        // Said out loud: the list is short because Dagster could not be
        // reached, not because those pipelines stopped existing.
        <p className="rounded-md bg-amber-500/5 px-3 py-2 text-xs text-amber-600 dark:text-amber-400">
          Dagster jobs are not listed — the orchestrator could not be
          reached ({state.data.error}). Pipelines authored in the console
          are shown below.
        </p>
      ) : null}
      {state.status === "success" && state.data.dagsterJobs?.supported === false ? (
        // A tenant-scoped caller refused the shared, un-tenanted Dagster
        // half — said honestly rather than showing a shorter list that
        // looks complete.
        <p className="rounded-md bg-amber-500/5 px-3 py-2 text-xs text-amber-600 dark:text-amber-400">
          Dagster jobs are not shown: {state.data.dagsterJobs.reason}
        </p>
      ) : null}

      {state.status === "success" && pipelines.length === 0 ? (
        <EmptyState
          title="No pipelines"
          description="Create a pipeline."
          action={
            <Button size="sm" render={<Link href="/pipelines/create" />}>
              Create Pipeline
            </Button>
          }
        />
      ) : null}

      {state.status === "success" && pipelines.length > 0 ? (
        <DataTable table={table}>
          <DataTableAdvancedToolbar
            table={table}
            onRefresh={state.reload}
            exportName="Pipelines"
          >
            <DataTableSearch placeholder="Search name, source, target, owner…" />
          </DataTableAdvancedToolbar>
          <div className="flex flex-wrap items-center gap-1">
            <span className="mr-1 text-xs text-muted-foreground">Status</span>
            <Button
              size="sm"
              variant={activeStatuses.length === 0 ? "secondary" : "ghost"}
              render={
                <Link href={statusHref(removeFilter(tableUrlState.filters, "status"))} />
              }
            >
              All
            </Button>
            {QUICK_STATUSES.map((status) => {
              const active = activeStatuses.includes(status.value)
              return (
                <Button
                  key={status.value}
                  size="sm"
                  variant={active ? "secondary" : "ghost"}
                  aria-pressed={active}
                  render={
                    <Link
                      href={statusHref(
                        toggleFilterValue(tableUrlState.filters, "status", status.value)
                      )}
                    />
                  }
                >
                  {status.label}
                </Button>
              )
            })}
          </div>
        </DataTable>
      ) : null}

      <AgenticBuilderDialog
        open={agenticOpen}
        onOpenChange={setAgenticOpen}
        onCreated={() => state.reload()}
      />
    </div>
  )
}
