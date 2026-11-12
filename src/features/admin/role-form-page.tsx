"use client"

import * as React from "react"
import Link from "next/link"
import { useParams, useRouter, useSearchParams } from "next/navigation"
import { FormPage } from "@/components/patterns/form-page"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { identityService, type Role, type RoleTableAccess, type RoleTableGrant, type SetRoleTableAccessInput } from "@/services"
import { PermissionPicker } from "./permission-picker"
import {
  grantsUnrestricted,
  RoleTableAccessSection,
  type TableAccessDraft,
} from "./role-table-access"

/**
 * The schema.table names a set of grants covers, compared without regard to
 * case as the rule compares them (`table_access.rs` `allows`). One name per
 * grant, sorted so a different order is not a change; duplicates collapse
 * because the rule reads the grants as a union.
 */
function grantNames(grants: readonly RoleTableGrant[]): string {
  return [...new Set(grants.map((grant) => `${grant.schema}.${grant.table}`.toLowerCase()))]
    .sort()
    .join("\n")
}

/**
 * Whether the draft differs from what the page loaded (`GOV-7` S2 review 1
 * S2). Every table-access write is audited and replaces the stored grants, so
 * an untouched section must not write: a role rename would otherwise record a
 * table-access change nobody made, and revert a change another administrator
 * made meanwhile. The comparison is of what the save would send: a scope of
 * `all` sends no grants.
 */
function tableAccessChanged(draft: TableAccessDraft, loaded: RoleTableAccess): boolean {
  if (draft.scope !== loaded.scope) return true
  const sent = draft.scope === "listed" ? draft.grants : []
  const stored = loaded.scope === "listed" ? loaded.grants : []
  return grantNames(sent) !== grantNames(stored)
}

/**
 * The three fields a role has, shared by Create and Edit (`WEB-3` S3-D8).
 * `GOV-7` S2 adds a section to the Edit page; `GOV-8` D2 puts the same
 * section on Create too, so a role's data access is set when the role is
 * made. It belongs in the same body, below these fields, so the two pages
 * cannot drift apart.
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

/**
 * `/admin/roles/new` (`WEB-7` D2): the create form as its own page.
 *
 * `GOV-8` D2: the page owns the role's data access in the same section Edit
 * role uses, so a new role can be narrowed as it is created rather than
 * opened a second time. The role is created first, then its tables are a
 * second request carrying the id the create returned, sent only when the
 * draft differs from "All tables". `GOV-8` D3: when that second request
 * fails the role exists and reads every table, and the page hands the
 * administrator the role's Edit page with the reason, because a retry here
 * would create a second role or be refused for the name.
 */
export function RoleCreatePage() {
  const router = useRouter()
  const backHref = "/admin/roles"
  const [name, setName] = React.useState("")
  const [permissions, setPermissions] = React.useState("")
  const [description, setDescription] = React.useState("")
  // `GOV-8` D2: a new role starts reading every table, which is the API's
  // own default, so an untouched section writes nothing.
  const [tableAccess, setTableAccess] = React.useState<TableAccessDraft>({
    scope: "all",
    grants: [],
  })
  const create = useServiceAction(
    withNotify(
      { success: "Role created", error: "Failed to create role" },
      (signal, input: Parameters<typeof identityService.createRole>[0]) =>
        identityService.createRole(input, signal)
    )
  )
  // `GOV-8` D3: the failed second write is reported on the Edit page, not by
  // a toast here, so this action is deliberately not wrapped in `withNotify`.
  const saveAccess = useServiceAction(
    (signal, id: string, input: SetRoleTableAccessInput) =>
      identityService.setRoleTableAccess(id, input, signal)
  )
  // `GOV-8` D2 copies Edit role's `*:*` rule (`GOV-7` D4): the API refuses to
  // narrow such a role, so the section is disabled and creation sends no
  // table-access write.
  const fullAccess = grantsUnrestricted(permissions)

  async function handleCreate() {
    const created = await create.run({
      name: name.trim(),
      permissions: permissions.trim(),
      description: description.trim(),
    })
    if (!created) return
    // `GOV-8` D2: the test for "the tables changed" is Edit role's own
    // `tableAccessChanged`, against the draft a new role starts from.
    if (
      fullAccess ||
      !tableAccessChanged(tableAccess, { scope: "all", grants: [] })
    ) {
      router.push(backHref)
      return
    }
    const accessSaved = await saveAccess.run(created.id, {
      scope: tableAccess.scope,
      grants: tableAccess.scope === "listed" ? tableAccess.grants : [],
    })
    if (accessSaved) {
      router.push(backHref)
      return
    }
    // `GOV-8` D3: the role exists and reads every table until the tables are
    // saved on its Edit page, which says so while `?tables=unsaved` is there.
    router.push(`/admin/roles/${encodeURIComponent(created.id)}/edit?tables=unsaved`)
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
      submitting={create.status === "pending" || saveAccess.status === "pending"}
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
      <RoleTableAccessSection
        idPrefix="role"
        value={tableAccess}
        onChange={setTableAccess}
        disabledReason={fullAccess ? "A role with full access cannot be narrowed" : null}
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
  const searchParams = useSearchParams()
  // `GOV-8` D3: Create role sends the administrator here when the role was
  // created but its tables were not saved, so this page says what happened
  // and what to do. The draft is not carried over: the administrator picks
  // the tables again on this page.
  const tablesUnsaved = searchParams.get("tables") === "unsaved"
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
  // `GOV-7` S2 review 1 S2: what was stored is also what the draft is
  // compared against, so a Save that did not touch this section sends no
  // table-access request at all.
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
  const renamed = name.trim() !== role.name
  // The API refuses to narrow a role that holds `*:*` (GOV-7 D4), so the
  // section is disabled with the server's own sentence and no table-access
  // write is sent for such a role. Whatever grants it already stores stay:
  // the API refuses to replace them, and they apply again if the role ever
  // loses `*:*`.
  const fullAccess = grantsUnrestricted(permissions)
  const loadingAccess = accessState.status === "loading"

  async function handleSave() {
    // The role first, then its tables: the table-access write is a separate
    // request, so either can fail on its own and the page says which.
    const saved = await update.run({
      name: name.trim(),
      permissions: permissions.trim(),
      description: description.trim(),
    })
    if (!saved) return
    // `GOV-7` S2 review 1 S2: only a section that changed writes. A `*:*`
    // role cannot be narrowed, and a section that could not be loaded is an
    // untouched section: the role save completes and the page goes back.
    if (
      fullAccess ||
      !access ||
      !tableAccess ||
      !tableAccessChanged(tableAccess, access)
    ) {
      router.push(backHref)
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
      {tablesUnsaved ? (
        <div
          role="status"
          className="rounded-md border border-amber-500/40 bg-amber-500/5 p-3"
        >
          {/* `GOV-8` D3: the exact sentence the plan fixes, so the reader
              knows the role exists, reads every table, and what to do. */}
          <p className="text-sm text-amber-700 dark:text-amber-400">
            This role was created, but its tables were not saved. It reads every table
            until you save them here.
          </p>
        </div>
      ) : null}
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
