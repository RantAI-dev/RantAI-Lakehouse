"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DataTableSkeleton } from "@/components/data-table/data-table-skeleton"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { formatNumber, formatRelativeTime } from "@/lib/format"
import { queryService } from "@/services"
import type { CollaborationProject } from "@/services/contracts/queries"
import { getCollaborationColumns } from "./collaboration-columns"
import { QueryStudioTabs } from "./query-studio-tabs"

function CollaborationDrawerContent({
  project,
}: {
  readonly project: CollaborationProject
}) {
  return (
    <>
      <p className="text-sm">{project.description}</p>
      <MetadataList
        items={[
          { label: "Members", value: formatNumber(project.members) },
          { label: "Updated", value: formatRelativeTime(project.updatedAt) },
        ]}
      />
      {/* Honest about the shape of the data: `members` is a count set once
          at creation, and nothing links queries to a project yet. Saying
          so beats a drawer that looks like it lost its content. */}
      <p className="text-xs text-muted-foreground">
        Member names and project queries are not stored yet — a project is
        currently a shared label, not a workspace.
      </p>
    </>
  )
}

export function CollaborationPage() {
  const state = useService((s) => queryService.listCollaboration(s), [])
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

  const columns = React.useMemo(
    () =>
      getCollaborationColumns({
        onInspect: (project) => setSelected(project),
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
          (r) => r.description,
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
    persistKey: "/query-studio/collaboration",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

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
        description="Shared query projects. A project records its name, member count and description today — queries are not attached to one yet."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <PlusIcon data-icon="inline-start" />
            Create Project
          </Button>
        }
      />
      <QueryStudioTabs />
      {state.status === "loading" ? (
        <DataTableSkeleton columnCount={5} rowCount={6} filterCount={1} />
      ) : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
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
        <div className="space-y-4">
          <DataTableAdvancedToolbar
            table={table}
            onRefresh={state.reload}
            exportName="Collaboration projects"
          >
            <DataTableSearch placeholder="Search projects..." />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}
      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? "Project"}
      >
        {selected ? <CollaborationDrawerContent project={selected} /> : null}
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
