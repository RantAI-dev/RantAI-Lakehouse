"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableFacetedFilter } from "@/components/data-table/data-table-faceted-filter"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { Pill, StatusBadge } from "@/components/patterns/status-badge"
import { useDataTable } from "@/hooks/use-data-table"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { ENTITY_STATUS_LABEL } from "@/lib/status"
import { agentService } from "@/services"
import type { AgentWorkflow } from "@/services/contracts/agents"
import { getWorkflowColumns } from "./workflow-columns"

function ApprovalGatePill({ required }: { readonly required: boolean }) {
  return required ? (
    <Pill tone="warning">Approval gate</Pill>
  ) : (
    <Pill tone="neutral">Autonomous</Pill>
  )
}

export function WorkflowsPage() {
  const state = useService((s) => agentService.listWorkflows(s), [])
  const [selected, setSelected] = React.useState<AgentWorkflow | null>(null)

  const columns = React.useMemo(
    () => getWorkflowColumns({ onSelect: setSelected }),
    []
  )

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

  const statusOptions = React.useMemo(() => {
    const present = new Set(state.data?.map((r) => r.status) ?? [])
    return [...present].map((s) => ({ value: s, label: ENTITY_STATUS_LABEL[s] }))
  }, [state.data])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Agent Workflows"
        description="Visual and declarative agent pipelines with triggers, tools, and approval gates."
        actions={
          <Button size="sm" render={<Link href="/agents/workflows/create" />}>
            <PlusIcon data-icon="inline-start" />
            Create Workflow
          </Button>
        }
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" && (state.data?.length ?? 0) === 0 ? (
        <EmptyState
          title="No workflows"
          description="Create an agent workflow with triggers, tools, and approval gates."
          action={
            <Button size="sm" render={<Link href="/agents/workflows/create" />}>
              Create Workflow
            </Button>
          }
        />
      ) : null}
      {state.status === "success" && (state.data?.length ?? 0) > 0 ? (
        <DataTable
          table={table}
          renderToolbar={() => (
            <DataTableAdvancedToolbar table={table}>
              <DataTableSearch
                table={table}
                placeholder="Search workflows..."
                className="w-full sm:w-64"
              />
              {table.getColumn("status") && (
                <DataTableFacetedFilter
                  column={table.getColumn("status")}
                  title="Status"
                  options={statusOptions}
                />
              )}
            </DataTableAdvancedToolbar>
          )}
        />
      ) : null}
      <DetailDrawer
        open={selected != null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description="Workflow detail"
      >
        {selected ? (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge status={selected.status} />
              <ApprovalGatePill required={selected.approvalRequired} />
            </div>
            <MetadataList
              items={[
                { label: "Owner", value: selected.owner },
                { label: "Trigger", value: selected.trigger },
                { label: "Steps", value: selected.steps },
                {
                  label: "Last run",
                  value: formatRelativeTime(selected.lastRunAt),
                },
              ]}
            />
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}
