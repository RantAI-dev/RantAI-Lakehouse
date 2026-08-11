"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { Pill, StatusBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { ENTITY_STATUS_LABEL } from "@/lib/status"
import { agentService } from "@/services"
import type { AgentWorkflow } from "@/services/contracts/agents"

function ApprovalGatePill({ required }: { required: boolean }) {
  return required ? (
    <Pill tone="warning">Approval gate</Pill>
  ) : (
    <Pill tone="neutral">Autonomous</Pill>
  )
}

const columns: ColumnDef<AgentWorkflow>[] = [
  { key: "name", header: "Workflow", render: (r) => r.name },
  { key: "status", header: "Status", render: (r) => <StatusBadge status={r.status} /> },
  { key: "trigger", header: "Trigger", render: (r) => r.trigger },
  { key: "steps", header: "Steps", render: (r) => r.steps },
  { key: "approval", header: "Approval", render: (r) => <ApprovalGatePill required={r.approvalRequired} /> },
  { key: "last", header: "Last run", render: (r) => formatRelativeTime(r.lastRunAt) },
  { key: "owner", header: "Owner", render: (r) => r.owner },
]

export function WorkflowsPage() {
  const state = useService((s) => agentService.listWorkflows(s), [])
  const [search, setSearch] = React.useState("")
  const [status, setStatus] = React.useState("all")
  const [selected, setSelected] = React.useState<AgentWorkflow | null>(null)

  const statusOptions = React.useMemo(() => {
    const present = new Set(state.data?.map((r) => r.status) ?? [])
    return [...present].map((s) => ({ value: s, label: ENTITY_STATUS_LABEL[s] }))
  }, [state.data])

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (status !== "all" && r.status !== status) return false
      if (!q) return true
      return [r.name, r.owner, r.trigger].some((v) => v.toLowerCase().includes(q))
    })
  }, [state.data, search, status])

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
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, owner, trigger..."
        />
        <FilterSelect
          value={status}
          onChange={setStatus}
          options={statusOptions}
          allLabel="All statuses"
          ariaLabel="Filter by status"
        />
      </FilterToolbar>
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
          columns={columns}
          rows={filtered}
          rowKey={(r) => r.id}
          onRowClick={setSelected}
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
