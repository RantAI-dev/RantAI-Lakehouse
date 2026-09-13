"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { formatRelativeTime } from "@/lib/format"
import { knowledgeService } from "@/services"
import type { VectorJob } from "@/services/contracts/knowledge"
import { getVectorJobColumns } from "./vector-job-columns"

function VectorJobDrawerContent({ job }: { readonly job: VectorJob }) {
  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <StatusBadge status={job.status} />
      </div>
      <MetadataList
        items={[
          {
            label: "Source",
            value: <span className="font-mono text-xs">{job.source}</span>,
          },
          { label: "Embedding model", value: job.embeddingModel },
          { label: "Index type", value: job.indexType },
          { label: "Owner", value: job.owner },
          {
            label: "Last run",
            value: formatRelativeTime(job.lastRunAt),
          },
        ]}
      />
      <div className="flex flex-wrap gap-2">
        {job.sourceId ? (
          <Button
            size="sm"
            variant="ghost"
            render={<Link href={`/knowledge?source=${encodeURIComponent(job.sourceId)}`} />}
          >
            Knowledge source
          </Button>
        ) : null}
        {job.outputAssetId ? (
          <Button
            size="sm"
            variant="ghost"
            render={<Link href={`/data/assets/${job.outputAssetId}`} />}
          >
            Output asset
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          render={<Link href={`/semantic-search?source=${encodeURIComponent(job.source)}`} />}
        >
          Try in Semantic Search
        </Button>
      </div>
    </>
  )
}

export function VectorJobsPage() {
  const state = useService((s) => knowledgeService.listVectorJobs(s), [])
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

  const columns = React.useMemo(
    () =>
      getVectorJobColumns({
        onInspect: (job) => setSelected(job),
      }),
    []
  )

  const rawData = React.useMemo(() => state.data ?? [], [state.data])

  const { table } = useDataTable({
    data: rawData,
    columns,
    pageCount: 1,
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
    shallow: false,
    clearOnDefault: true,
  })

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

  const canSubmit = Boolean(
    name.trim() && source.trim() && embeddingModel.trim() && indexType.trim()
  )

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
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch placeholder="Search name, source, owner..." />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}
      <DetailDrawer
        open={selected != null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description="Vector job detail"
      >
        {selected ? <VectorJobDrawerContent job={selected} /> : null}
      </DetailDrawer>
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="New Vector Job"
        description="Schedule embedding and indexing for a knowledge source."
        canSubmit={canSubmit}
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
