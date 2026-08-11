"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { StatusBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { ENTITY_STATUS_LABEL } from "@/lib/status"
import { pipelineService } from "@/services"
import type { Pipeline, PipelineKind } from "@/services/contracts/pipelines"

const KIND_OPTIONS: { value: PipelineKind; label: string }[] = [
  { value: "batch", label: "Batch" },
  { value: "incremental", label: "Incremental" },
  { value: "document", label: "Document" },
  { value: "vector", label: "Vector" },
]

const columns: ColumnDef<Pipeline>[] = [
  { key: "name", header: "Pipeline", render: (r) => (
    <div><p className="font-medium">{r.name}</p><p className="text-xs text-muted-foreground">{r.kind}</p></div>
  )},
  { key: "status", header: "Status", render: (r) => <StatusBadge status={r.status} /> },
  { key: "owner", header: "Owner", render: (r) => r.owner },
  { key: "source", header: "Source", render: (r) => r.source },
  { key: "target", header: "Target", render: (r) => r.target },
  { key: "schedule", header: "Schedule", render: (r) => r.schedule },
  { key: "last", header: "Last run", render: (r) => formatRelativeTime(r.lastRunAt) },
  { key: "next", header: "Next run", render: (r) => (r.nextRunAt ? formatRelativeTime(r.nextRunAt) : "—") },
  { key: "fresh", header: "Freshness", render: (r) => <FreshnessIndicator lagSeconds={r.freshnessLagSeconds} /> },
  { key: "sla", header: "SLA", render: (r) => (r.slaOk ? "OK" : "Breached") },
]

export function PipelinesPage() {
  const router = useRouter()
  const state = useService((s) => pipelineService.listPipelines(s), [])
  const [search, setSearch] = React.useState("")
  const [kind, setKind] = React.useState("all")
  const [status, setStatus] = React.useState("all")

  const statusOptions = React.useMemo(() => {
    const present = new Set(state.data?.map((p) => p.status) ?? [])
    return [...present].map((s) => ({ value: s, label: ENTITY_STATUS_LABEL[s] }))
  }, [state.data])

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((p) => {
      if (kind !== "all" && p.kind !== kind) return false
      if (status !== "all" && p.status !== status) return false
      if (!q) return true
      return [p.name, p.source, p.target, p.owner].some((v) =>
        v.toLowerCase().includes(q)
      )
    })
  }, [state.data, search, kind, status])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Pipelines"
        description="Batch, incremental, document, and vector flows with run health and freshness."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, source, target, owner..."
        />
        <FilterSelect
          value={kind}
          onChange={setKind}
          options={KIND_OPTIONS}
          allLabel="All kinds"
          ariaLabel="Filter by kind"
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
      {state.status === "success" ? (
        <DataTable
          columns={columns}
          rows={filtered}
          rowKey={(r) => r.id}
          onRowClick={(r) => router.push(`/pipelines/${r.id}`)}
        />
      ) : null}
    </div>
  )
}
