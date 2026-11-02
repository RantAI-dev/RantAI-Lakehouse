"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"

import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { formatRelativeTime } from "@/lib/format"
import { identityService } from "@/services"
import type { Role, Tenant, User } from "@/services"
import { getUserColumns, PillList, UserStatusPill } from "./user-columns"

export function UsersPage() {
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("identity:write")
  const state = useService((s) => identityService.listUsers(s), [])
  // ADM-4 D7: the role editor needs the roles that exist, and the tenant
  // remover needs the tenant ids behind the tenant NAMES a user carries.
  const rolesState = useService((s) => identityService.listRoles(s), [])
  const tenantsState = useService((s) => identityService.listTenants(s), [])
  const [selected, setSelected] = React.useState<User | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [email, setEmail] = React.useState("")
  const [roles, setRoles] = React.useState("")
  const [tenants, setTenants] = React.useState("")
  // ADM-3 D1/D6: the temporary password the server generated, shown in
  // the dialog below exactly ONCE. It is never persisted to
  // localStorage, sessionStorage, or any service-layer cache entry — it
  // is kept as local component state and cleared when the dialog closes,
  // so a re-render of the table (a filter keystroke, a refetch landing)
  // cannot leak it through a hook that stores its argument.
  const [revealedPassword, setRevealedPassword] = React.useState<string | null>(null)
  // ADM-4 D7: the user whose roles are being edited, and the role names
  // the editor currently has ticked.
  const [rolesTarget, setRolesTarget] = React.useState<User | null>(null)
  const [roleSelection, setRoleSelection] = React.useState<string[]>([])
  // ADM-4 D7: the user a tenant is being taken away from.
  const [tenantTarget, setTenantTarget] = React.useState<User | null>(null)
  // ADM-4 D7: the user whose status is being switched, and the dialog
  // that asks for confirmation first.
  const [statusTarget, setStatusTarget] = React.useState<User | null>(null)
  const tableUrlState = useTableUrlState()

  const create = useServiceAction(
    (signal, input: Parameters<typeof identityService.inviteUser>[0]) =>
      identityService.inviteUser(input, signal)
  )

  const reset = useServiceAction(
    (signal, id: string) => identityService.resetPassword(id, signal)
  )

  // ADM-4 D1: replace the user's local role set.
  const setRolesAction = useServiceAction(
    (signal, id: string, roleNames: string[]) =>
      identityService.setUserRoles(id, roleNames, signal)
  )

  // ADM-4 D2: take a user out of a tenant.
  const removeTenant = useServiceAction(
    (signal, id: string, tenantId: string) =>
      identityService.removeUserFromTenant(id, tenantId, signal)
  )

  // ADM-4 D3: switch a user off, and back on again.
  const setStatus = useServiceAction((signal, id: string, active: boolean) =>
    active ? identityService.reactivateUser(id, signal) : identityService.deactivateUser(id, signal)
  )

  // `state.reload` is stable (it is a `useCallback` inside `useService`),
  // so it can be a dependency without re-creating the callbacks below on
  // every render.
  const reloadUsers = state.reload

  const handleResetPassword = React.useCallback(
    async (user: User) => {
      // `useServiceAction.run` resolves to `null` on any thrown error (the
      // service layer classifies upstream failures into ServiceError; see
      // use-service.ts:99-105). A failed reset MUST NOT open the dialog —
      // the `if (result)` guard is the single source of truth for "the
      // reset actually returned a password".
      const result = await reset.run(user.id)
      if (!result) return
      setRevealedPassword(result.temporaryPassword)
    },
    [reset]
  )

  const openRoleEditor = React.useCallback((user: User) => {
    setRolesTarget(user)
    setRoleSelection([...user.roles])
  }, [])

  const columns = React.useMemo(
    () =>
      getUserColumns({
        onSelect: setSelected,
        canWrite,
        onResetPassword: (user) => void handleResetPassword(user),
        onEditRoles: openRoleEditor,
        onRemoveFromTenant: (user) => setTenantTarget(user),
        onToggleStatus: (user) => setStatusTarget(user),
      }),
    [canWrite, handleResetPassword, openRoleEditor]
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
      // ADM-3 D1: the server generated a temporary password and returned
      // it once. Show it, and only here — the server never sends it
      // again.
      setRevealedPassword(result.temporaryPassword)
      state.reload()
    }
  }

  async function handleSaveRoles() {
    if (!rolesTarget) return
    // A failed save leaves the dialog open with the selection intact, so
    // the admin can try again without re-picking every role.
    const result = await setRolesAction.run(rolesTarget.id, roleSelection)
    if (!result) return
    setRolesTarget(null)
    reloadUsers()
  }

  async function handleRemoveFromTenant(user: User, tenant: Tenant) {
    const result = await removeTenant.run(user.id, tenant.id)
    if (!result) return
    setTenantTarget(null)
    reloadUsers()
  }

  async function handleToggleStatus() {
    if (!statusTarget) return
    const active = statusTarget.status !== "active"
    const result = await setStatus.run(statusTarget.id, active)
    if (!result) return
    setStatusTarget(null)
    reloadUsers()
  }

  const availableRoles: Role[] = rolesState.status === "success" ? rolesState.data : []
  const knownTenants: Tenant[] = tenantsState.status === "success" ? tenantsState.data : []

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
      {/* ADM-3 D1/D6: the temporary password, shown exactly once. The
          server keeps only the `Argon2id` hash, so it cannot be shown
          again — the admin has to hand it over now. */}
      <Dialog
        open={revealedPassword !== null}
        onOpenChange={(open) => {
          if (!open) setRevealedPassword(null)
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Temporary password</DialogTitle>
            <DialogDescription>
              Shown once. The user must change it at first sign-in.
            </DialogDescription>
          </DialogHeader>
          <code
            data-testid="revealed-password"
            className="block break-all rounded bg-muted p-2 text-xs"
          >
            {revealedPassword}
          </code>
          <div className="flex justify-end">
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                if (revealedPassword === null) return
                void navigator.clipboard.writeText(revealedPassword)
              }}
            >
              Copy
            </Button>
          </div>
        </DialogContent>
      </Dialog>

      {/* ADM-4 D1/D7: the role editor. The ticked roles ARE the user's
          whole local role set — a role left unticked is a role the user
          no longer holds. */}
      <Dialog
        open={rolesTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRolesTarget(null)
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Edit roles</DialogTitle>
            <DialogDescription>
              The roles {rolesTarget?.name ?? "this user"} holds. A role left
              unticked is a role the user no longer holds.
            </DialogDescription>
          </DialogHeader>
          <div className="max-h-64 overflow-y-auto">
            {rolesState.status === "loading" ? (
              <p className="text-sm text-muted-foreground">Loading roles…</p>
            ) : null}
            {rolesState.status === "success" && availableRoles.length === 0 ? (
              <p className="text-sm text-muted-foreground">No roles are defined yet.</p>
            ) : null}
            <ul className="grid gap-1">
              {availableRoles.map((role) => (
                <li
                  key={role.id}
                  className="flex items-center gap-2 rounded-md border border-border px-2 py-1.5 text-sm"
                >
                  <Checkbox
                    checked={roleSelection.includes(role.name)}
                    aria-label={role.name}
                    onCheckedChange={(checked) =>
                      setRoleSelection((current) =>
                        checked === true
                          ? [...current, role.name]
                          : current.filter((name) => name !== role.name)
                      )
                    }
                  />
                  <span className="min-w-0 flex-1 truncate">{role.name}</span>
                </li>
              ))}
            </ul>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setRolesTarget(null)}>
              Cancel
            </Button>
            <Button
              type="button"
              disabled={setRolesAction.status === "pending"}
              onClick={() => void handleSaveRoles()}
            >
              {setRolesAction.status === "pending" ? "Saving…" : "Save roles"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* ADM-4 D2/D7: taking a user out of a tenant. The user's tenant
          NAMES are what the directory shows, so the id behind each one
          is looked up in the tenant list. */}
      <Dialog
        open={tenantTarget !== null}
        onOpenChange={(open) => {
          if (!open) setTenantTarget(null)
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Remove from tenant</DialogTitle>
            <DialogDescription>
              The tenant {tenantTarget?.name ?? "this user"} should leave.
            </DialogDescription>
          </DialogHeader>
          {tenantTarget !== null && tenantTarget.tenants.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              This user is in no tenant.
            </p>
          ) : null}
          <ul className="grid gap-1">
            {(tenantTarget?.tenants ?? []).map((tenantName) => {
              const tenant = knownTenants.find((candidate) => candidate.name === tenantName)
              return (
                <li
                  key={tenantName}
                  className="flex items-center justify-between gap-2 rounded-md border border-border px-2 py-1.5 text-sm"
                >
                  <span className="min-w-0 flex-1 truncate">{tenantName}</span>
                  <Button
                    type="button"
                    size="xs"
                    variant="outline"
                    disabled={!tenant || removeTenant.status === "pending"}
                    onClick={() => {
                      if (!tenant || !tenantTarget) return
                      void handleRemoveFromTenant(tenantTarget, tenant)
                    }}
                  >
                    Remove
                  </Button>
                </li>
              )
            })}
          </ul>
        </DialogContent>
      </Dialog>

      {/* ADM-4 D3/D7: switching a user off ends every session they hold,
          so the console asks first. */}
      <ConfirmActionDialog
        open={statusTarget !== null}
        onOpenChange={(open) => {
          if (!open) setStatusTarget(null)
        }}
        title={
          statusTarget?.status === "active" ? "Deactivate this user?" : "Reactivate this user?"
        }
        description={
          statusTarget?.status === "active"
            ? `${statusTarget?.name ?? "This user"} will not be able to sign in.`
            : `${statusTarget?.name ?? "This user"} will be able to sign in again.`
        }
        impact={
          statusTarget?.status === "active"
            ? "Every session they hold ends now. Their data, dashboards and audit history stay."
            : undefined
        }
        confirmLabel={statusTarget?.status === "active" ? "Deactivate" : "Reactivate"}
        destructive={statusTarget?.status === "active"}
        confirming={setStatus.status === "pending"}
        onConfirm={() => void handleToggleStatus()}
      />
    </div>
  )
}
