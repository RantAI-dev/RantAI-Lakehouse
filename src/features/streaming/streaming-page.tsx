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
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { StatusBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import {
  formatBytes,
  formatLagSeconds,
  formatRate,
  formatRelativeTime,
} from "@/lib/format"
import { ENTITY_STATUS_LABEL } from "@/lib/status"
import { streamingService } from "@/services"
import type { StreamingJob } from "@/services/contracts/streaming"

const columns: ColumnDef<StreamingJob>[] = [
  { key: "name", header: "Job", render: (r) => <span className="font-mono text-sm">{r.name}</span> },
  { key: "status", header: "Status", render: (r) => <StatusBadge status={r.status} /> },
  { key: "lag", header: "Lag", render: (r) => formatLagSeconds(r.lagSeconds) },
  { key: "tp", header: "Throughput", render: (r) => formatRate(r.throughputPerSec) },
  { key: "state", header: "State size", render: (r) => formatBytes(r.stateSizeBytes) },
  { key: "barrier", header: "Last barrier", render: (r) => formatRelativeTime(r.lastBarrierAt) },
  { key: "owner", header: "Owner", render: (r) => r.owner },
]

export function StreamingPage() {
  const router = useRouter()
  const state = useService((s) => streamingService.listJobs(s), [])
  const [search, setSearch] = React.useState("")
  const [status, setStatus] = React.useState("all")

  const statusOptions = React.useMemo(() => {
    const present = new Set(state.data?.map((j) => j.status) ?? [])
    return [...present].map((s) => ({ value: s, label: ENTITY_STATUS_LABEL[s] }))
  }, [state.data])

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((j) => {
      if (status !== "all" && j.status !== status) return false
      if (!q) return true
      return (
        j.name.toLowerCase().includes(q) ||
        j.owner.toLowerCase().includes(q) ||
        j.sources.some((src) => src.toLowerCase().includes(q))
      )
    })
  }, [state.data, search, status])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Streaming Jobs"
        description="Real-time materialized views, lag, throughput, and agent triggers."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, owner, sources..."
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
          onRowClick={(r) => router.push(`/streaming/${r.id}`)}
        />
      ) : null}
    </div>
  )
}
