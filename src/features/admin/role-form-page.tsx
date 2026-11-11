"use client"

import * as React from "react"
import Link from "next/link"
import { useParams, useRouter } from "next/navigation"
import { FormPage } from "@/components/patterns/form-page"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { identityService, type Role, type SetRoleTableAccessInput } from "@/services"
import { PermissionPicker } from "./permission-picker"
import {
  grantsUnrestricted,
  RoleTableAccessSection,
  type TableAccessDraft,
} from "./role-table-access"

/**
 * The three fields a role has, shared by Create and Edit (`WEB-3` S3-D8).
 * `GOV-7` S2 adds a section to the Edit page; it belongs in the same body,
 * below these fields, so both pages cannot drift apart.
 */
export function RoleFields({
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
        <PermissionPicker id={`${idPrefix}-perms`} value={permissions} onChange={onPermissions} />
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

/** `/admin/roles/new` (`WEB-7` D2): the create form as its own page. */
export function RoleCreatePage() {
  const router = useRouter()
  const backHref = "/admin/roles"
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

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      permissions: permissions.trim(),
      description: description.trim(),
    })
    if (result) router.push(backHref)
  }

  return (
    <FormPage
      title="Create Role"
      description="Define a role template with permissions."
      backHref={backHref}
      backLabel="Back to Roles"
      submitLabel="Create"
      submittingLabel="Creating…"
      canSubmit={Boolean(name.trim() && permissions.trim() && description.trim())}
      submitting={create.status === "pending"}
      error={create.status === "error" ? create.error.message : null}
      onSubmit={() => void handleCreate()}
      onCancel={() => router.push(backHref)}
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
    </FormPage>
  )
}

/**
 * `/admin/roles/[id]/edit` (`WEB-7` D2/D4): the edit form, loaded by id
 * through the existing list read so a reload works. An id that names no
 * role shows the not-found state instead of an empty form.
 */
export function RoleEditPage() {
  const { id } = useParams<{ id: string }>()
  const state = useService((s) => identityService.listRoles(s), [])
  if (state.status === "loading") return <LoadingSkeleton rows={6} />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  const role = state.data.find((candidate) => candidate.id === id)
  if (!role) {
    return (
      <EmptyState
        title="Role not found"
        description="This role was deleted, or the address names one that never existed."
        action={
          <Button variant="outline" size="sm" render={<Link href="/admin/roles" />}>
            Back to Roles
          </Button>
        }
      />
    )
  }
  return <RoleEditForm role={role} />
}

/**
 * Edit one role (`PUT /api/identity/roles/{id}`). Access rules name roles
 * by name, so the API refuses a rename while one of them names this role
 * (`WEB-3` review R2); a changed name says that before saving, rather than
 * letting the save come back a 409 the person did not expect. A new
 * description or permission list is unaffected.
 */
function RoleEditForm({ role }: { role: Role }) {
  const router = useRouter()
  const backHref = "/admin/roles"
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
  // GOV-7 D8: the role's tables belong to the same Save. The draft starts
  // from what the API stored, so a grant the catalog does not list still
  // travels back in the PUT until the person removes it.
  const accessState = useService(
    (signal) => identityService.getRoleTableAccess(role.id, signal),
    [role.id]
  )
  const [tableAccessDraft, setTableAccessDraft] = React.useState<TableAccessDraft | null>(null)
  const access = accessState.status === "success" ? accessState.data : null
  const tableAccess: TableAccessDraft | null = React.useMemo(
    () => tableAccessDraft ?? (access ? { scope: access.scope, grants: access.grants } : null),
    [tableAccessDraft, access]
  )
  const saveAccess = useServiceAction(
    withNotify(
      { success: "Table access saved", error: "Failed to save table access" },
      (signal, input: SetRoleTableAccessInput) =>
        identityService.setRoleTableAccess(role.id, input, signal)
    )
  )
  const [accessSkipped, setAccessSkipped] = React.useState(false)
  const renamed = name.trim() !== role.name
  // The API refuses to narrow a role that holds `*:*` (GOV-7 D4), so the
  // section is disabled with the server's own sentence and no table-access
  // write is sent for such a role. Whatever grants it already stores stay:
  // the API refuses to replace them, and they apply again if the role ever
  // loses `*:*`.
  const fullAccess = grantsUnrestricted(permissions)
  const loadingAccess = accessState.status === "loading"

  async function handleSave() {
    setAccessSkipped(false)
    // The role first, then its tables: the table-access write is a separate
    // request, so either can fail on its own and the page says which.
    const saved = await update.run({
      name: name.trim(),
      permissions: permissions.trim(),
      description: description.trim(),
    })
    if (!saved) return
    if (fullAccess) {
      // A `*:*` role reads every table whatever its stored grants say; the
      // API refuses the narrowing write, and the section is disabled.
      router.push(backHref)
      return
    }
    if (!tableAccess) {
      // Its stored grants never arrived: writing now would replace grants
      // nobody saw. The role is saved and the page says what was left.
      setAccessSkipped(true)
      return
    }
    const accessSaved = await saveAccess.run({
      scope: tableAccess.scope,
      grants: tableAccess.scope === "listed" ? tableAccess.grants : [],
    })
    if (accessSaved) router.push(backHref)
  }

  const saveError =
    update.status === "error"
      ? `The role was not saved: ${update.error.message}`
      : saveAccess.status === "error"
        ? `The role was saved, but its tables were not: ${saveAccess.error.message}`
        : accessSkipped
          ? "The role was saved. Its tables were not saved because they could not be loaded; retry above and save again."
          : null

  return (
    <FormPage
      title="Edit Role"
      description={`Change ${role.name}.`}
      backHref={backHref}
      backLabel="Back to Roles"
      submitLabel="Save"
      submittingLabel="Saving…"
      canSubmit={Boolean(name.trim() && permissions.trim() && description.trim()) && !loadingAccess}
      submitting={update.status === "pending" || saveAccess.status === "pending"}
      error={saveError}
      onSubmit={() => void handleSave()}
      onCancel={() => router.push(backHref)}
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
      <RoleTableAccessSection
        idPrefix="edit-role"
        value={tableAccess}
        onChange={setTableAccessDraft}
        disabledReason={fullAccess ? "A role with full access cannot be narrowed" : null}
        accessError={
          accessState.status === "error"
            ? { message: accessState.error.message, retry: accessState.reload }
            : null
        }
      />
      {renamed && name.trim() ? (
        <p className="text-sm text-muted-foreground">
          Access rules name roles by name. A rename is refused while an access rule names{" "}
          {role.name}; change those rules first, or keep the name.
        </p>
      ) : null}
    </FormPage>
  )
}
