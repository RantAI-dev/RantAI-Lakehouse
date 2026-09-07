"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { formatRelativeTime } from "@/lib/format"
import { ENTITY_STATUS_LABEL } from "@/lib/status"
import { knowledgeService } from "@/services"
import type { VectorJob } from "@/services/contracts/knowledge"

const columns: ColumnDef<VectorJob>[] = [
  { key: "name", header: "Job", render: (r) => r.name },
  { key: "status", header: "Status", render: (r) => <StatusBadge status={r.status} /> },
  { key: "source", header: "Source", className: "font-mono text-xs", render: (r) => r.source },
  { key: "model", header: "Model", render: (r) => r.embeddingModel },
  { key: "index", header: "Index", render: (r) => r.indexType },
  { key: "last", header: "Last run", render: (r) => formatRelativeTime(r.lastRunAt) },
  { key: "owner", header: "Owner", render: (r) => r.owner },
]

export function VectorJobsPage() {
  const state = useService((s) => knowledgeService.listVectorJobs(s), [])
  const [search, setSearch] = React.useState("")
  const [status, setStatus] = React.useState("all")
  const [selected, setSelected] = React.useState<VectorJob | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [source, setSource] = React.useState("")
  const [embeddingModel, setEmbeddingModel] = React.useState("")
  const [indexType, setIndexType] = React.useState("")
  const create = useServiceAction(
    withNotify(
      { success: "Vector job queued", error: "Failed to queue vector job" },
      (signal, input: Parameters<typeof knowledgeService.createVectorJob>[0]) =>
        knowledgeService.createVectorJob(input, signal)
    )
  )

  const statusOptions = React.useMemo(() => {
    const present = new Set(state.data?.map((r) => r.status) ?? [])
    return [...present].map((s) => ({ value: s, label: ENTITY_STATUS_LABEL[s] }))
  }, [state.data])

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (status !== "all" && r.status !== status) return false
      if (!q) return true
      return [r.name, r.source, r.owner].some((v) => v.toLowerCase().includes(q))
    })
  }, [state.data, search, status])

  function resetForm() {
    setName("")
    setSource("")
    setEmbeddingModel("")
    setIndexType("")
  }

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      source: source.trim(),
      embeddingModel: embeddingModel.trim(),
      indexType: indexType.trim(),
    })
    if (result) {
      setCreateOpen(false)
      resetForm()
      state.reload()
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Vector & Knowledge Jobs"
        description="Embedding, indexing, and refresh jobs for the AI tier."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <PlusIcon data-icon="inline-start" />
            New Vector Job
          </Button>
        }
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, source, owner..."
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
        description="Vector job detail"
      >
        {selected ? (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge status={selected.status} />
            </div>
            <MetadataList
              items={[
                {
                  label: "Source",
                  value: <span className="font-mono text-xs">{selected.source}</span>,
                },
                { label: "Embedding model", value: selected.embeddingModel },
                { label: "Index type", value: selected.indexType },
                { label: "Owner", value: selected.owner },
                {
                  label: "Last run",
                  value: formatRelativeTime(selected.lastRunAt),
                },
              ]}
            />
            <div className="flex flex-wrap gap-2">
              {selected.sourceId ? (
                <Button
                  size="sm"
                  variant="ghost"
                  render={<Link href="/knowledge" />}
                >
                  Knowledge source
                </Button>
              ) : null}
              {selected.outputAssetId ? (
                <Button
                  size="sm"
                  variant="ghost"
                  render={<Link href={`/data/assets/${selected.outputAssetId}`} />}
                >
                  Output asset
                </Button>
              ) : null}
              <Button
                size="sm"
                variant="ghost"
                render={<Link href="/semantic-search" />}
              >
                Try in Semantic Search
              </Button>
            </div>
          </>
        ) : null}
      </DetailDrawer>
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="New Vector Job"
        description="Schedule embedding and indexing for a knowledge source."
        canSubmit={Boolean(
          name.trim() && source.trim() && embeddingModel.trim() && indexType.trim()
        )}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="vj-name">Name</Label>
          <Input id="vj-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="vj-source">Source</Label>
          <Input
            id="vj-source"
            value={source}
            onChange={(e) => setSource(e.target.value)}
            placeholder="knowledge://policy-handbook"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="vj-model">Embedding model</Label>
          <Input
            id="vj-model"
            value={embeddingModel}
            onChange={(e) => setEmbeddingModel(e.target.value)}
            placeholder="text-embedding-3-large"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="vj-index">Index type</Label>
          <Input
            id="vj-index"
            value={indexType}
            onChange={(e) => setIndexType(e.target.value)}
            placeholder="hnsw"
          />
        </div>
      </CreateSheet>
    </div>
  )
}
