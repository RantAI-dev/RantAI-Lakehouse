"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useAuth } from "@/features/auth/auth-provider"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService, useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { identityService } from "@/services"
import type { Role } from "@/services/contracts/identity"
import { getRoleColumns } from "./roles-columns"

/**
 * Roles (`WEB-3` S3-D8, was "Teams & Roles"): create, edit and delete a
 * role, each with `identity:write`. Creating and editing are their own
 * pages (`WEB-7` D2); deleting stays a confirmation on this list. Deleting
 * a role people still hold, or a built-in role, is refused by the API with
 * a 409 whose sentence the dialog shows as it is (S3a-D2).
 */
export function RolesPage() {
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("identity:write")
  const router = useRouter()
  const state = useService((s) => identityService.listRoles(s), [])
  const [deleting, setDeleting] = React.useState<Role | null>(null)

  const columns = React.useMemo(
    () =>
      canWrite
        ? getRoleColumns({
            onEdit: (role) => router.push(`/admin/roles/${encodeURIComponent(role.id)}/edit`),
            onDelete: setDeleting,
          })
        : getRoleColumns(),
    [canWrite, router]
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

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Roles"
        description="What each role may do, and how many people hold it."
        actions={
          <Button
            size="sm"
            render={<Link href="/admin/roles/new" />}
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
    </div>
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
