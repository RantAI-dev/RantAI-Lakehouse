"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { StatusBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { ENTITY_STATUS_LABEL } from "@/lib/status"
import { governanceService } from "@/services"
import type { Policy } from "@/services/contracts/governance"

const columns: ColumnDef<Policy>[] = [
  { key: "name", header: "Policy", render: (r) => r.name },
  { key: "status", header: "Status", render: (r) => <StatusBadge status={r.status} /> },
  { key: "kind", header: "Kind", render: (r) => r.kind },
  { key: "subjects", header: "Subjects", render: (r) => r.subjects },
  { key: "resources", header: "Resources", render: (r) => r.resources },
  { key: "effect", header: "Effect", render: (r) => r.effect },
  { key: "ver", header: "Version", render: (r) => `v${r.version}` },
  { key: "owner", header: "Owner", render: (r) => r.owner },
  { key: "updated", header: "Updated", render: (r) => formatRelativeTime(r.updatedAt) },
]

export function PoliciesPage() {
  const state = useService((s) => governanceService.listPolicies(s), [])
  const [search, setSearch] = React.useState("")
  const [status, setStatus] = React.useState("all")
  const [kind, setKind] = React.useState("all")
  const [selected, setSelected] = React.useState<Policy | null>(null)

  const statusOptions = React.useMemo(() => {
    const present = new Set(state.data?.map((r) => r.status) ?? [])
    return [...present].map((s) => ({ value: s, label: ENTITY_STATUS_LABEL[s] }))
  }, [state.data])

  const kindOptions = React.useMemo(() => {
    const present = new Set(state.data?.map((r) => r.kind) ?? [])
    return [...present].map((k) => ({ value: k, label: k }))
  }, [state.data])

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (status !== "all" && r.status !== status) return false
      if (kind !== "all" && r.kind !== kind) return false
      if (!q) return true
      return [r.name, r.subjects, r.resources].some((v) =>
        v.toLowerCase().includes(q)
      )
    })
  }, [state.data, search, status, kind])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Policies"
        description="Access, agent, residency, retention, and budget policies."
        actions={
          <Button size="sm" render={<Link href="/governance/policies/create" />}>
            <PlusIcon data-icon="inline-start" />
            Create Policy
          </Button>
        }
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, subjects, resources..."
        />
        <FilterSelect
          value={status}
          onChange={setStatus}
          options={statusOptions}
          allLabel="All statuses"
          ariaLabel="Filter by status"
        />
        <FilterSelect
          value={kind}
          onChange={setKind}
          options={kindOptions}
          allLabel="All kinds"
          ariaLabel="Filter by kind"
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" && (state.data?.length ?? 0) === 0 ? (
        <EmptyState
          title="No policies"
          description="Create an access, agent, residency, retention, or budget policy."
          action={
            <Button size="sm" render={<Link href="/governance/policies/create" />}>
              Create Policy
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
        open={selected != null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description="Policy detail"
      >
        {selected ? (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge status={selected.status} />
            </div>
            <MetadataList
              items={[
                { label: "Kind", value: selected.kind },
                { label: "Subjects", value: selected.subjects },
                { label: "Resources", value: selected.resources },
                { label: "Effect", value: selected.effect },
                { label: "Version", value: `v${selected.version}` },
                { label: "Owner", value: selected.owner },
                {
                  label: "Updated",
                  value: formatRelativeTime(selected.updatedAt),
                },
              ]}
            />
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}
