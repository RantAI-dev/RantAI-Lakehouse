"use client"

import * as React from "react"
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
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { identityService } from "@/services"
import type { User } from "@/services/contracts/identity"

const STATUS_OPTIONS = [
  { value: "active", label: "Active" },
  { value: "inactive", label: "Inactive" },
]

function UserStatusPill({ status }: { status: User["status"] }) {
  return status === "active" ? (
    <Pill tone="success">Active</Pill>
  ) : (
    <Pill tone="neutral">Inactive</Pill>
  )
}

function PillList({ values }: { values: string[] }) {
  if (values.length === 0) return <span>—</span>
  return (
    <div className="flex flex-wrap gap-1">
      {values.map((v) => (
        <Pill key={v} tone="neutral">
          {v}
        </Pill>
      ))}
    </div>
  )
}

const columns: ColumnDef<User>[] = [
  {
    key: "name",
    header: "User",
    render: (r) => (
      <div>
        <p className="font-medium">{r.name}</p>
        <p className="text-xs text-muted-foreground">{r.email}</p>
      </div>
    ),
  },
  {
    key: "status",
    header: "Status",
    render: (r) => <UserStatusPill status={r.status} />,
  },
  { key: "roles", header: "Roles", render: (r) => <PillList values={r.roles} /> },
  {
    key: "tenants",
    header: "Tenants",
    render: (r) => <PillList values={r.tenants} />,
  },
  {
    key: "last",
    header: "Last activity",
    render: (r) => formatRelativeTime(r.lastActivity),
  },
]

export function UsersPage() {
  const state = useService((s) => identityService.listUsers(s), [])
  const [search, setSearch] = React.useState("")
  const [status, setStatus] = React.useState("all")
  const [role, setRole] = React.useState("all")
  const [selected, setSelected] = React.useState<User | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [email, setEmail] = React.useState("")
  const [roles, setRoles] = React.useState("")
  const [tenants, setTenants] = React.useState("")
  const create = useServiceAction(
    (signal, input: Parameters<typeof identityService.inviteUser>[0]) =>
      identityService.inviteUser(input, signal)
  )

  const roleOptions = React.useMemo(() => {
    const present = new Set((state.data ?? []).flatMap((u) => u.roles))
    return [...present].sort().map((r) => ({ value: r, label: r }))
  }, [state.data])

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((u) => {
      if (status !== "all" && u.status !== status) return false
      if (role !== "all" && !u.roles.includes(role)) return false
      if (!q) return true
      return [u.name, u.email].some((v) => v.toLowerCase().includes(q))
    })
  }, [state.data, search, status, role])

  function resetForm() {
    setName("")
    setEmail("")
    setRoles("")
    setTenants("")
  }

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      email: email.trim(),
      roles: roles
        .split(",")
        .map((r) => r.trim())
        .filter(Boolean),
      tenants: tenants
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
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
        title="Users"
        description="People, roles, tenant membership, and recent activity."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <PlusIcon data-icon="inline-start" />
            Invite User
          </Button>
        }
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, email..."
        />
        <FilterSelect
          value={status}
          onChange={setStatus}
          options={STATUS_OPTIONS}
          allLabel="All statuses"
          ariaLabel="Filter by status"
        />
        <FilterSelect
          value={role}
          onChange={setRole}
          options={roleOptions}
          allLabel="All roles"
          ariaLabel="Filter by role"
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
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
        title={selected?.name ?? ""}
        description={selected?.email}
      >
        {selected ? (
          <MetadataList
            items={[
              { label: "Email", value: selected.email },
              {
                label: "Status",
                value: <UserStatusPill status={selected.status} />,
              },
              { label: "Roles", value: <PillList values={selected.roles} /> },
              {
                label: "Tenants",
                value: <PillList values={selected.tenants} />,
              },
              {
                label: "Last activity",
                value: formatRelativeTime(selected.lastActivity),
              },
            ]}
          />
        ) : null}
      </DetailDrawer>
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Invite User"
        description="Invite a user with roles and tenant membership."
        canSubmit={Boolean(name.trim() && email.trim())}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
        submitLabel="Invite"
      >
        <div className="space-y-1.5">
          <Label htmlFor="user-name">Name</Label>
          <Input id="user-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="user-email">Email</Label>
          <Input
            id="user-email"
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="user-roles">Roles (comma-separated)</Label>
          <Input
            id="user-roles"
            value={roles}
            onChange={(e) => setRoles(e.target.value)}
            placeholder="Analyst, Viewer"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="user-tenants">Tenants (comma-separated)</Label>
          <Input
            id="user-tenants"
            value={tenants}
            onChange={(e) => setTenants(e.target.value)}
            placeholder="acme, demo"
          />
        </div>
      </CreateSheet>
    </div>
  )
}
