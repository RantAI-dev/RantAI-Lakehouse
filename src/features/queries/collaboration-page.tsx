"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { FilterToolbar, SearchField } from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
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
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [collaborators, setCollaborators] = React.useState("")
  const [formError, setFormError] = React.useState<string | null>(null)
  const create = useServiceAction(
    withNotify(
      { success: "Project created", error: "Failed to create project" },
      (signal, input: Parameters<typeof queryService.createCollaborationProject>[0]) =>
        queryService.createCollaborationProject(input, signal)
    )
  )

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return state.data ?? []
    return (state.data ?? []).filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        r.description.toLowerCase().includes(q)
    )
  }, [state.data, search])

  function resetForm() {
    setName("")
    setCollaborators("")
    setFormError(null)
  }

  async function handleCreate() {
    const collabs = collaborators
      .split(",")
      .map((c) => c.trim())
      .filter(Boolean)
    if (!name.trim()) {
      setFormError("Name is required.")
      return
    }
    if (collabs.length < 1) {
      setFormError("At least one collaborator is required.")
      return
    }
    setFormError(null)
    const result = await create.run({ name: name.trim(), collaborators: collabs })
    if (result) {
      setCreateOpen(false)
      resetForm()
      state.reload()
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Collaboration"
        description="Shared query projects, members, and activity."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <PlusIcon data-icon="inline-start" />
            Create Project
          </Button>
        }
      />
      <QueryStudioTabs />
      <FilterToolbar>
        <SearchField value={search} onChange={setSearch} placeholder="Search projects..." />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" && (state.data?.length ?? 0) === 0 ? (
        <EmptyState
          title="No projects"
          description="Create a shared query project to collaborate with your team."
          action={
            <Button size="sm" onClick={() => setCreateOpen(true)}>
              Create Project
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
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Create Project"
        description="Name and collaborators for a shared query workspace."
        canSubmit={Boolean(name.trim() && collaborators.trim())}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
        error={create.status === "error" ? create.error.message : formError}
      >
        <div className="space-y-1.5">
          <Label htmlFor="col-name">Name</Label>
          <Input id="col-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="col-members">Collaborators (comma-separated)</Label>
          <Input
            id="col-members"
            value={collaborators}
            onChange={(e) => setCollaborators(e.target.value)}
            placeholder="rina@rantai.id, bayu@rantai.id"
          />
          {collaborators.trim() ? (
            <div className="flex flex-wrap gap-1 pt-1">
              {collaborators.split(",").map((c) => c.trim()).filter(Boolean).map((c) => (
                <span key={c} className="rounded-full border border-border px-2 py-0.5 text-xs">
                  {c}
                </span>
              ))}
            </div>
          ) : null}
        </div>
      </CreateSheet>
    </div>
  )
}
