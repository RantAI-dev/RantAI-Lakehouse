"use client"

import * as React from "react"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { FilterToolbar, SearchField } from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService } from "@/hooks/use-service"
import { formatNumber, formatRelativeTime } from "@/lib/format"
import { queryService } from "@/services"
import type { CollaborationProject } from "@/services/contracts/queries"
import { QueryStudioTabs } from "./query-studio-tabs"

const columns: ColumnDef<CollaborationProject>[] = [
  { key: "name", header: "Project", render: (r) => <span className="font-medium">{r.name}</span> },
  { key: "desc", header: "Description", render: (r) => r.description },
  { key: "members", header: "Members", render: (r) => formatNumber(r.members) },
  { key: "updated", header: "Updated", render: (r) => formatRelativeTime(r.updatedAt) },
]

export function CollaborationPage() {
  const state = useService((s) => queryService.listCollaboration(s), [])
  const [search, setSearch] = React.useState("")
  const [selected, setSelected] = React.useState<CollaborationProject | null>(null)

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return state.data ?? []
    return (state.data ?? []).filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        r.description.toLowerCase().includes(q)
    )
  }, [state.data, search])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Collaboration"
        description="Shared query projects, members, and activity."
      />
      <QueryStudioTabs />
      <FilterToolbar>
        <SearchField value={search} onChange={setSearch} placeholder="Search projects..." />
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
        title={selected?.name ?? "Project"}
      >
        {selected ? (
          <>
            <p className="text-sm">{selected.description}</p>
            <MetadataList
              items={[
                { label: "Members", value: formatNumber(selected.members) },
                { label: "Updated", value: formatRelativeTime(selected.updatedAt) },
              ]}
            />
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}
