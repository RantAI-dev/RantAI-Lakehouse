"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import {
  ClassificationBadge,
  StatusBadge,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { formatCompactNumber, formatRelativeTime } from "@/lib/format"
import {
  CLASSIFICATION_LABEL,
  type Classification,
} from "@/lib/status"
import { knowledgeService } from "@/services"
import type {
  KnowledgeSource,
  KnowledgeSourceKind,
} from "@/services/contracts/knowledge"
import {
  getKnowledgeColumns,
  IndexStatusPill,
} from "./knowledge-columns"

const KIND_OPTIONS: { value: KnowledgeSourceKind; label: string }[] = [
  { value: "file", label: "File" },
  { value: "object-storage", label: "Object storage" },
  { value: "web", label: "Web" },
  { value: "table", label: "Table" },
  { value: "query", label: "Query" },
  { value: "manual", label: "Manual" },
]

const CLASSIFICATION_OPTIONS = (
  Object.keys(CLASSIFICATION_LABEL) as Classification[]
).map((c) => ({ value: c, label: CLASSIFICATION_LABEL[c] }))

const selectClassName =
  "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

function KnowledgeDrawerContent({ source }: { readonly source: KnowledgeSource }) {
  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <StatusBadge status={source.status} />
        <IndexStatusPill status={source.indexStatus} />
        <FreshnessIndicator lagSeconds={source.freshnessLagSeconds} />
      </div>
      <MetadataList
        items={[
          { label: "Kind", value: source.kind },
          { label: "Owner", value: source.owner },
          { label: "Version", value: source.version },
          { label: "Embedding model", value: source.embeddingModel },
          { label: "Chunks", value: formatCompactNumber(source.chunkCount) },
          {
            label: "Classification",
            value: (
              <ClassificationBadge classification={source.classification} />
            ),
          },
          { label: "Dependent agents", value: source.dependentAgents },
          {
            label: "Last refresh",
            value: formatRelativeTime(source.lastRefresh),
          },
        ]}
      />
      <div className="flex flex-wrap gap-2">
        {source.vectorJobId ? (
          <Button
            size="sm"
            variant="ghost"
            render={<Link href={`/vector-jobs?job=${encodeURIComponent(source.vectorJobId)}`} />}
          >
            Vector job {source.vectorJobId}
          </Button>
        ) : null}
        {source.assetId ? (
          <Button
            size="sm"
            variant="ghost"
            render={<Link href={`/data/assets/${source.assetId}`} />}
          >
            Catalog asset
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          render={<Link href={`/semantic-search?source=${encodeURIComponent(source.name)}`} />}
        >
          Try in Semantic Search
        </Button>
      </div>
    </>
  )
}

export function KnowledgePage() {
  const state = useService((s) => knowledgeService.listSources(s), [])
  const [selected, setSelected] = React.useState<KnowledgeSource | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [formKind, setFormKind] = React.useState<KnowledgeSourceKind>("file")
  const [embeddingModel, setEmbeddingModel] = React.useState("")
  const [classification, setClassification] =
    React.useState<Classification>("internal")

  const create = useServiceAction(
    withNotify(
      { success: "Knowledge source created", error: "Failed to create source" },
      (signal, input: Parameters<typeof knowledgeService.createSource>[0]) =>
        knowledgeService.createSource(input, signal)
    )
  )

  const columns = React.useMemo(
    () =>
      getKnowledgeColumns({
        onInspect: (source) => setSelected(source),
      }),
    []
  )

  const rawData = React.useMemo(() => state.data ?? [], [state.data])

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(rawData, {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.name,
          (r) => r.owner,
        ],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [rawData, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator]
  )

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/knowledge",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  function resetForm() {
    setName("")
    setFormKind("file")
    setEmbeddingModel("")
    setClassification("internal")
  }

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      kind: formKind,
      embeddingModel: embeddingModel.trim(),
      classification,
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
        title="Knowledge"
        description="Governed knowledge sources with versions, embeddings, freshness, and dependent agents."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <PlusIcon data-icon="inline-start" />
            Add Source
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
            <DataTableSearch placeholder="Search name or owner..." />
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
        description="Knowledge source detail"
      >
        {selected ? <KnowledgeDrawerContent source={selected} /> : null}
      </DetailDrawer>
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Add Source"
        description="Register a governed knowledge source for embedding and retrieval."
        canSubmit={Boolean(name.trim() && embeddingModel.trim())}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="ks-name">Name</Label>
          <Input id="ks-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="ks-kind">Kind</Label>
          <select
            id="ks-kind"
            className={selectClassName}
            value={formKind}
            onChange={(e) => setFormKind(e.target.value as KnowledgeSourceKind)}
          >
            {KIND_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="ks-model">Embedding model</Label>
          <Input
            id="ks-model"
            value={embeddingModel}
            onChange={(e) => setEmbeddingModel(e.target.value)}
            placeholder="text-embedding-3-large"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="ks-class">Classification</Label>
          <select
            id="ks-class"
            className={selectClassName}
            value={classification}
            onChange={(e) => setClassification(e.target.value as Classification)}
          >
            {CLASSIFICATION_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        </div>
      </CreateSheet>
    </div>
  )
}
