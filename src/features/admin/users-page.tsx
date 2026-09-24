"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"

import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { formatRelativeTime } from "@/lib/format"
import { identityService } from "@/services"
import type { User } from "@/services/contracts/identity"
import { getUserColumns, PillList, UserStatusPill } from "./user-columns"

export function UsersPage() {
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("identity:write")
  const state = useService((s) => identityService.listUsers(s), [])
  const [selected, setSelected] = React.useState<User | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [email, setEmail] = React.useState("")
  const [roles, setRoles] = React.useState("")
  const [tenants, setTenants] = React.useState("")
  const tableUrlState = useTableUrlState()

  const create = useServiceAction(
    (signal, input: Parameters<typeof identityService.inviteUser>[0]) =>
      identityService.inviteUser(input, signal)
  )

  const columns = React.useMemo(
    () => getUserColumns({ onSelect: setSelected }),
    []
  )

  const filteredData = React.useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (u) => u.name,
        (u) => u.email,
        (u) => u.roles.join(" "),
        (u) => u.tenants.join(" "),
      ],
      filters: tableUrlState.filters,
      joinOperator: tableUrlState.joinOperator,
    })
  }, [
    state.status,
    state.data,
    tableUrlState.search,
    tableUrlState.filters,
    tableUrlState.joinOperator,
  ])

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/admin/users",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

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
          <Button
            size="sm"
            onClick={() => setCreateOpen(true)}
            disabled={!canWrite}
            title={canWrite ? undefined : "You don't have permission to invite users."}
          >
            <PlusIcon data-icon="inline-start" />
            Invite User
          </Button>
        }
      />

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable table={table}>
          <DataTableAdvancedToolbar table={table}>
            <DataTableSearch placeholder="Search name, email, roles, tenants…" />
          </DataTableAdvancedToolbar>
        </DataTable>
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
                value:
                  selected.lastActivity === null
                    ? "Not recorded"
                    : formatRelativeTime(selected.lastActivity),
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
