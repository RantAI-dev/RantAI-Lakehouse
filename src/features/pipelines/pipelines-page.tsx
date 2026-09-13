"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { PlusIcon, SparklesIcon } from "lucide-react"

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
import { useService } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { pipelineService } from "@/services"
import { AgenticBuilderDialog } from "./agentic-builder-dialog"
import { getPipelineColumns } from "./pipeline-columns"

export function PipelinesPage() {
  const router = useRouter()
  const state = useService((s) => pipelineService.listPipelines(s), [])
  const [agenticOpen, setAgenticOpen] = React.useState(false)
  const tableUrlState = useTableUrlState()

  const columns = React.useMemo(
    () =>
      getPipelineColumns({
        onView: (p) => router.push(`/pipelines/${p.id}`),
      }),
    [router]
  )

  const filteredData = React.useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
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
    persistKey: "/pipelines",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Pipelines"
        description="Batch, incremental, document, and vector flows with run health and freshness."
        actions={
          <>
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

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && (state.data?.length ?? 0) === 0 ? (
        <EmptyState
          title="No pipelines"
          description="Create a pipeline or generate one with the Agentic Builder."
          action={
            <Button size="sm" render={<Link href="/pipelines/create" />}>
              Create Pipeline
            </Button>
          }
        />
      ) : null}
      {state.status === "success" && (state.data?.length ?? 0) > 0 ? (
        <DataTable table={table}>
          <DataTableAdvancedToolbar table={table}>
            <DataTableSearch
              placeholder="Search name, source, target, owner…"
            />
          </DataTableAdvancedToolbar>
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
