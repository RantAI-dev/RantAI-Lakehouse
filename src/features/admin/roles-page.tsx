"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService, useServiceAction } from "@/hooks/use-service"
import { notifySuccess, withNotify } from "@/lib/notify"
import { identityService } from "@/services"
import type { Role } from "@/services/contracts/identity"
import { getRoleColumns } from "./roles-columns"
import { PermissionPicker } from "./permission-picker"

/**
 * Roles (`WEB-3` S3-D8, was "Teams & Roles"): create, edit and delete a
 * role, each with `identity:write`. Deleting a role people still hold, or
 * a built-in role, is refused by the API with a 409 whose sentence the
 * dialog shows as it is (S3a-D2).
 */
export function RolesPage() {
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("identity:write")
  const state = useService((s) => identityService.listRoles(s), [])
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [permissions, setPermissions] = React.useState("")
  const [description, setDescription] = React.useState("")
  const create = useServiceAction(
    withNotify(
      { success: "Role created", error: "Failed to create role" },
      (signal, input: Parameters<typeof identityService.createRole>[0]) =>
        identityService.createRole(input, signal)
    )
  )

  const [editing, setEditing] = React.useState<Role | null>(null)
  const [deleting, setDeleting] = React.useState<Role | null>(null)

  const columns = React.useMemo(
    () => (canWrite ? getRoleColumns({ onEdit: setEditing, onDelete: setDeleting }) : getRoleColumns()),
    [canWrite]
  )

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(state.data ?? [], {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.name,
          (r) => r.description,
          (r) => r.permissions,
        ],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [state.data, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator]
  )

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/admin/roles",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

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
        title="Roles"
        description="What each role may do, and how many people hold it."
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
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch
              placeholder="Search name, description, permissions..."
            />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}
      {editing ? (
        <EditRoleSheet
          role={editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null)
            state.reload()
          }}
        />
      ) : null}
      {deleting ? (
        <DeleteRoleDialog
          role={deleting}
          onClose={() => setDeleting(null)}
          onDeleted={() => {
            setDeleting(null)
            state.reload()
          }}
        />
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
        <RoleFields
          idPrefix="role"
          name={name}
          permissions={permissions}
          description={description}
          onName={setName}
          onPermissions={setPermissions}
          onDescription={setDescription}
        />
      </CreateSheet>
    </div>
  )
}

/** The three fields a role has, shared by Create and Edit. */
function RoleFields({
  idPrefix,
  name,
  permissions,
  description,
  onName,
  onPermissions,
  onDescription,
}: {
  idPrefix: string
  name: string
  permissions: string
  description: string
  onName: (v: string) => void
  onPermissions: (v: string) => void
  onDescription: (v: string) => void
}) {
  return (
    <>
      <div className="space-y-1.5">
        <Label htmlFor={`${idPrefix}-name`}>Name</Label>
        <Input id={`${idPrefix}-name`} value={name} onChange={(e) => onName(e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor={`${idPrefix}-perms`}>Permissions</Label>
        <PermissionPicker
          id={`${idPrefix}-perms`}
          value={permissions}
          onChange={onPermissions}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor={`${idPrefix}-desc`}>Description</Label>
        <Input
          id={`${idPrefix}-desc`}
          value={description}
          onChange={(e) => onDescription(e.target.value)}
        />
      </div>
    </>
  )
}

/**
 * Edit one role (`PUT /api/identity/roles/{id}`). Access rules name roles
 * by name, so the API refuses a rename while one of them names this role
 * (`WEB-3` review R2); a changed name says that before saving, rather than
 * letting the save come back a 409 the person did not expect. A new
 * description or permission list is unaffected.
 */
function EditRoleSheet({
  role,
  onClose,
  onSaved,
}: {
  role: Role
  onClose: () => void
  onSaved: () => void
}) {
  const [name, setName] = React.useState(role.name)
  const [permissions, setPermissions] = React.useState(role.permissions)
  const [description, setDescription] = React.useState(role.description)
  const update = useServiceAction(
    withNotify(
      { success: "Role saved", error: "Failed to save role" },
      (signal, input: Parameters<typeof identityService.updateRole>[1]) =>
        identityService.updateRole(role.id, input, signal)
    )
  )
  const renamed = name.trim() !== role.name

  async function handleSave() {
    const saved = await update.run({
      name: name.trim(),
      permissions: permissions.trim(),
      description: description.trim(),
    })
    if (saved) onSaved()
  }

  return (
    <CreateSheet
      open
      onOpenChange={(open) => (open ? undefined : onClose())}
      title="Edit Role"
      description={`Change ${role.name}.`}
      submitLabel="Save"
      canSubmit={Boolean(name.trim() && permissions.trim() && description.trim())}
      submitting={update.status === "pending"}
      error={update.status === "error" ? update.error.message : null}
      onSubmit={handleSave}
    >
      <RoleFields
        idPrefix="edit-role"
        name={name}
        permissions={permissions}
        description={description}
        onName={setName}
        onPermissions={setPermissions}
        onDescription={setDescription}
      />
      {renamed && name.trim() ? (
        <p className="text-sm text-muted-foreground">
          Access rules name roles by name. A rename is refused while an access rule names{" "}
          {role.name}; change those rules first, or keep the name.
        </p>
      ) : null}
    </CreateSheet>
  )
}

/**
 * Delete one role (`DELETE /api/identity/roles/{id}`). The API refuses a
 * role people still hold, or a built-in one, with a 409 and a fixed
 * sentence; the dialog stays open and shows it, read from the action's
 * state on the next render (not from the closure that awaited it).
 */
function DeleteRoleDialog({
  role,
  onClose,
  onDeleted,
}: {
  role: Role
  onClose: () => void
  onDeleted: () => void
}) {
  const remove = useServiceAction((signal) =>
    identityService.deleteRole(role.id, signal).then(() => true)
  )

  async function confirm() {
    const done = await remove.run()
    if (done === null) return
    notifySuccess(`Deleted role ${role.name}`)
    onDeleted()
  }

  return (
    <ConfirmActionDialog
      open
      onOpenChange={(open) => (open ? undefined : onClose())}
      title="Delete role"
      description={`Delete ${role.name}?`}
      impact={
        role.members > 0
          ? `${role.members} ${role.members === 1 ? "person holds" : "people hold"} it. A role can be deleted only once nobody holds it.`
          : "Nobody holds it. Its permissions go with it."
      }
      confirmLabel="Delete role"
      destructive
      confirming={remove.status === "pending"}
      onConfirm={() => void confirm()}
    >
      {remove.status === "error" ? (
        <p role="alert" className="text-sm text-destructive">
          {remove.error.message}
        </p>
      ) : null}
    </ConfirmActionDialog>
  )
}
