"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatNumber } from "@/lib/format"
import { identityService } from "@/services"
import type { Role } from "@/services/contracts/identity"
import { useAuth } from "@/features/auth/auth-provider"

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
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("identity:write")
  const state = useService((s) => identityService.listRoles(s), [])
  const [search, setSearch] = React.useState("")
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [permissions, setPermissions] = React.useState("")
  const [description, setDescription] = React.useState("")
  const create = useServiceAction(
    (signal, input: Parameters<typeof identityService.createRole>[0]) =>
      identityService.createRole(input, signal)
  )

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return state.data ?? []
    return (state.data ?? []).filter((r) =>
      [r.name, r.description, r.permissions].some((v) =>
        v.toLowerCase().includes(q)
      )
    )
  }, [state.data, search])

  function resetForm() {
    setName("")
    setPermissions("")
    setDescription("")
  }

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      permissions: permissions.trim(),
      description: description.trim(),
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
        title="Teams & Roles"
        description="Role templates, permissions, and membership."
        actions={
          <Button
            size="sm"
            onClick={() => setCreateOpen(true)}
            disabled={!canWrite}
            title={canWrite ? undefined : "You don't have permission to create roles."}
          >
            <PlusIcon data-icon="inline-start" />
            Create Role
          </Button>
        }
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
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Create Role"
        description="Define a role template with permissions."
        canSubmit={Boolean(name.trim() && permissions.trim() && description.trim())}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="role-name">Name</Label>
          <Input id="role-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="role-perms">Permissions</Label>
          <Input
            id="role-perms"
            value={permissions}
            onChange={(e) => setPermissions(e.target.value)}
            placeholder="catalog:read, query:run"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="role-desc">Description</Label>
          <Input
            id="role-desc"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </div>
      </CreateSheet>
    </div>
  )
}
