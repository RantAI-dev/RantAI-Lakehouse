"use client"

import * as React from "react"
import Link from "next/link"
import { CodeBlock } from "@/components/patterns/code-block"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { FilterToolbar, SearchField } from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { queryService } from "@/services"
import type { SavedQuery } from "@/services/contracts/queries"
import { QueryStudioTabs } from "./query-studio-tabs"

function TagPills({ tags }: { tags: string[] }) {
  return (
    <span className="flex flex-wrap gap-1">
      {tags.map((t) => (
        <Pill key={t} tone="neutral">
          {t}
        </Pill>
      ))}
    </span>
  )
}

const columns: ColumnDef<SavedQuery>[] = [
  { key: "title", header: "Title", render: (r) => <span className="font-medium">{r.title}</span> },
  { key: "owner", header: "Owner", render: (r) => r.owner },
  { key: "tags", header: "Tags", render: (r) => <TagPills tags={r.tags} /> },
  { key: "updated", header: "Updated", render: (r) => formatRelativeTime(r.updatedAt) },
]

export function SavedQueriesPage() {
  const state = useService((s) => queryService.listSaved(s), [])
  const [search, setSearch] = React.useState("")
  const [selected, setSelected] = React.useState<SavedQuery | null>(null)

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return state.data ?? []
    return (state.data ?? []).filter(
      (r) =>
        r.title.toLowerCase().includes(q) ||
        r.owner.toLowerCase().includes(q) ||
        r.tags.some((t) => t.toLowerCase().includes(q))
    )
  }, [state.data, search])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Saved Queries"
        description="Reusable SQL assets with owners and tags."
      />
      <QueryStudioTabs />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search title, tags, owner..."
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
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.title ?? "Saved query"}
        wide
      >
        {selected ? (
          <>
            <CodeBlock>{selected.sql}</CodeBlock>
            <MetadataList
              items={[
                { label: "Owner", value: selected.owner },
                { label: "Updated", value: formatRelativeTime(selected.updatedAt) },
                { label: "Tags", value: <TagPills tags={selected.tags} /> },
              ]}
            />
            <Button
              size="sm"
              render={<Link href={`/query-studio?saved=${selected.id}`} />}
            >
              Open in Studio
            </Button>
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}
