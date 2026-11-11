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
import { identityService } from "@/services"
import type { Role } from "@/services/contracts/identity"
import { PermissionPicker } from "./permission-picker"

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
  const renamed = name.trim() !== role.name

  async function handleSave() {
    const saved = await update.run({
      name: name.trim(),
      permissions: permissions.trim(),
      description: description.trim(),
    })
    if (saved) router.push(backHref)
  }

  return (
    <FormPage
      title="Edit Role"
      description={`Change ${role.name}.`}
      backHref={backHref}
      backLabel="Back to Roles"
      submitLabel="Save"
      submittingLabel="Saving…"
      canSubmit={Boolean(name.trim() && permissions.trim() && description.trim())}
      submitting={update.status === "pending"}
      error={update.status === "error" ? update.error.message : null}
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
      {renamed && name.trim() ? (
        <p className="text-sm text-muted-foreground">
          Access rules name roles by name. A rename is refused while an access rule names{" "}
          {role.name}; change those rules first, or keep the name.
        </p>
      ) : null}
    </FormPage>
  )
}
