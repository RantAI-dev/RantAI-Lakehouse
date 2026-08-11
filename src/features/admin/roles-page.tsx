"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService } from "@/hooks/use-service"
import { formatNumber } from "@/lib/format"
import { identityService } from "@/services"
import type { Role } from "@/services/contracts/identity"

const columns: ColumnDef<Role>[] = [
  { key: "name", header: "Role", render: (r) => r.name },
  {
    key: "members",
    header: "Members",
    render: (r) => formatNumber(r.members),
  },
  {
    key: "perms",
    header: "Permissions",
    render: (r) => <span className="font-mono text-xs">{r.permissions}</span>,
  },
  { key: "desc", header: "Description", render: (r) => r.description },
]

export function RolesPage() {
  const state = useService((s) => identityService.listRoles(s), [])
  const [search, setSearch] = React.useState("")

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return state.data ?? []
    return (state.data ?? []).filter((r) =>
      [r.name, r.description, r.permissions].some((v) =>
        v.toLowerCase().includes(q)
      )
    )
  }, [state.data, search])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Teams & Roles"
        description="Role templates, permissions, and membership."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, description, permissions..."
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable columns={columns} rows={filtered} rowKey={(r) => r.id} />
      ) : null}
    </div>
  )
}
