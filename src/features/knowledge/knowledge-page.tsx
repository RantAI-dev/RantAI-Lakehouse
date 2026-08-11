"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import {
  ClassificationBadge,
  Pill,
  StatusBadge,
} from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatCompactNumber, formatRelativeTime } from "@/lib/format"
import { ENTITY_STATUS_LABEL } from "@/lib/status"
import { knowledgeService } from "@/services"
import type {
  IndexStatus,
  KnowledgeSource,
  KnowledgeSourceKind,
} from "@/services/contracts/knowledge"

const INDEX_STATUS_TONE: Record<IndexStatus, "success" | "info" | "warning"> = {
  ready: "success",
  indexing: "info",
  degraded: "warning",
}

const INDEX_STATUS_LABEL: Record<IndexStatus, string> = {
  ready: "Ready",
  indexing: "Indexing",
  degraded: "Degraded",
}

function IndexStatusPill({ status }: { status: IndexStatus }) {
  return (
    <Pill tone={INDEX_STATUS_TONE[status]}>{INDEX_STATUS_LABEL[status]}</Pill>
  )
}

const KIND_OPTIONS: { value: KnowledgeSourceKind; label: string }[] = [
  { value: "file", label: "File" },
  { value: "object-storage", label: "Object storage" },
  { value: "web", label: "Web" },
  { value: "table", label: "Table" },
  { value: "query", label: "Query" },
  { value: "manual", label: "Manual" },
]

const columns: ColumnDef<KnowledgeSource>[] = [
  { key: "name", header: "Source", render: (r) => (
    <div><p className="font-medium">{r.name}</p><p className="text-xs text-muted-foreground">{r.kind} · {r.version}</p></div>
  )},
  { key: "status", header: "Status", render: (r) => <StatusBadge status={r.status} /> },
  { key: "index", header: "Index", render: (r) => <IndexStatusPill status={r.indexStatus} /> },
  { key: "owner", header: "Owner", render: (r) => r.owner },
  { key: "class", header: "Class", render: (r) => <ClassificationBadge classification={r.classification} /> },
  { key: "chunks", header: "Chunks", render: (r) => formatCompactNumber(r.chunkCount) },
  { key: "model", header: "Embedding", render: (r) => r.embeddingModel },
  { key: "fresh", header: "Freshness", render: (r) => <FreshnessIndicator lagSeconds={r.freshnessLagSeconds} /> },
  { key: "agents", header: "Agents", render: (r) => r.dependentAgents },
  { key: "refresh", header: "Last refresh", render: (r) => formatRelativeTime(r.lastRefresh) },
]

export function KnowledgePage() {
  const state = useService((s) => knowledgeService.listSources(s), [])
  const [search, setSearch] = React.useState("")
  const [kind, setKind] = React.useState("all")
  const [status, setStatus] = React.useState("all")
  const [selected, setSelected] = React.useState<KnowledgeSource | null>(null)

  const statusOptions = React.useMemo(() => {
    const present = new Set(state.data?.map((r) => r.status) ?? [])
    return [...present].map((s) => ({ value: s, label: ENTITY_STATUS_LABEL[s] }))
  }, [state.data])

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (kind !== "all" && r.kind !== kind) return false
      if (status !== "all" && r.status !== status) return false
      if (!q) return true
      return [r.name, r.owner].some((v) => v.toLowerCase().includes(q))
    })
  }, [state.data, search, kind, status])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Knowledge"
        description="Governed knowledge sources with versions, embeddings, freshness, and dependent agents."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name or owner..."
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
          onRowClick={setSelected}
        />
      ) : null}
      <DetailDrawer
        open={selected != null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description="Knowledge source detail"
      >
        {selected ? (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge status={selected.status} />
              <IndexStatusPill status={selected.indexStatus} />
              <FreshnessIndicator lagSeconds={selected.freshnessLagSeconds} />
            </div>
            <MetadataList
              items={[
                { label: "Kind", value: selected.kind },
                { label: "Owner", value: selected.owner },
                { label: "Version", value: selected.version },
                { label: "Embedding model", value: selected.embeddingModel },
                { label: "Chunks", value: formatCompactNumber(selected.chunkCount) },
                {
                  label: "Classification",
                  value: (
                    <ClassificationBadge classification={selected.classification} />
                  ),
                },
                { label: "Dependent agents", value: selected.dependentAgents },
                {
                  label: "Last refresh",
                  value: formatRelativeTime(selected.lastRefresh),
                },
              ]}
            />
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}
