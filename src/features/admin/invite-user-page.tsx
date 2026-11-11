"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { FormPage } from "@/components/patterns/form-page"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { authService, identityService } from "@/services"
import { rememberInvitePassword } from "./invite-password"

/**
 * `/admin/users/invite` (`WEB-7` D2/D3): the invite form, moved out of the
 * `CreateSheet` side panel onto its own page. Every field it had is here —
 * name, username (`ADM-11`), email, roles, tenants, the first-password mode
 * and the must-change flag.
 *
 * On success the list it came from shows the server-generated temporary
 * password in its one-shot dialog. The password crosses the redirect in
 * memory (`invite-password.ts`), never in the URL or web storage.
 */
export function InviteUserPage() {
  const router = useRouter()
  const backHref = "/admin/users"
  const rolesState = useService((s) => identityService.listRoles(s), [])
  const tenantsState = useService((s) => identityService.listTenants(s), [])
  const passwordRulesState = useService((s) => authService.passwordRules(s), [])
  const [name, setName] = React.useState("")
  const [username, setUsername] = React.useState("")
  const [email, setEmail] = React.useState("")
  const [selectedRoles, setSelectedRoles] = React.useState<string[]>([])
  const [selectedTenants, setSelectedTenants] = React.useState<string[]>([])
  const [passwordMode, setPasswordMode] = React.useState<"generate" | "set">("generate")
  const [setPassword, setSetPassword] = React.useState("")
  const [mustChangePassword, setMustChangePassword] = React.useState(true)

  const create = useServiceAction(
    (signal, input: Parameters<typeof identityService.inviteUser>[0]) =>
      identityService.inviteUser(input, signal)
  )

  async function handleCreate() {
    const passwordInput =
      passwordMode === "generate"
        ? { mode: "generate" as const }
        : { mode: "set" as const, value: setPassword }
    const result = await create.run({
      name: name.trim(),
      username: username.trim() || undefined,
      email: email.trim() || undefined,
      roles: selectedRoles,
      tenants: selectedTenants,
      password: passwordInput,
      mustChangePassword,
    })
    if (!result) return
    if (result.temporaryPassword) {
      rememberInvitePassword({
        password: result.temporaryPassword,
        mustChangePassword,
      })
    }
    router.push(backHref)
  }

  const availableRoles = rolesState.status === "success" ? rolesState.data : []
  const knownTenants = tenantsState.status === "success" ? tenantsState.data : []

  return (
    <FormPage
      title="Invite User"
      description="Invite a user with roles and tenant membership."
      backHref={backHref}
      backLabel="Back to Users"
      submitLabel="Invite"
      submittingLabel="Creating…"
      canSubmit={Boolean(
        name.trim() &&
          (username.trim() || email.trim()) &&
          (passwordMode === "generate" || setPassword.length >= 8)
      )}
      submitting={create.status === "pending"}
      error={create.status === "error" ? create.error.message : null}
      onSubmit={() => void handleCreate()}
      onCancel={() => router.push(backHref)}
    >
      <div className="space-y-1.5">
        <Label htmlFor="user-name">Name</Label>
        <Input id="user-name" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="user-username">Username</Label>
        <Input
          id="user-username"
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          placeholder="e.g. jdoe"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="user-email">Email (optional)</Label>
        <Input
          id="user-email"
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="jdoe@example.com"
        />
      </div>
      <div className="space-y-1.5">
        <Label>Roles</Label>
        <ul className="grid max-h-48 gap-1 overflow-y-auto rounded-md border border-border p-1">
          {availableRoles.length === 0 ? (
            <li className="px-2 py-1 text-xs text-muted-foreground">No roles defined</li>
          ) : null}
          {availableRoles.map((role) => (
            <li key={role.id} className="flex items-center gap-2 px-1.5 py-1 text-sm">
              <Checkbox
                id={`invite-role-${role.id}`}
                checked={selectedRoles.includes(role.name)}
                aria-label={role.name}
                onCheckedChange={(checked) =>
                  setSelectedRoles((current) =>
                    checked === true
                      ? [...current, role.name]
                      : current.filter((r) => r !== role.name)
                  )
                }
              />
              <label
                htmlFor={`invite-role-${role.id}`}
                className="flex-1 cursor-pointer truncate text-xs select-none"
              >
                {role.name}
              </label>
            </li>
          ))}
        </ul>
      </div>
      <div className="space-y-1.5">
        <Label>Tenants</Label>
        <ul className="grid max-h-48 gap-1 overflow-y-auto rounded-md border border-border p-1">
          {knownTenants.length === 0 ? (
            <li className="px-2 py-1 text-xs text-muted-foreground">No tenants defined</li>
          ) : null}
          {knownTenants.map((tenant) => (
            <li key={tenant.id} className="flex items-center gap-2 px-1.5 py-1 text-sm">
              <Checkbox
                id={`invite-tenant-${tenant.id}`}
                checked={selectedTenants.includes(tenant.name)}
                aria-label={tenant.name}
                onCheckedChange={(checked) =>
                  setSelectedTenants((current) =>
                    checked === true
                      ? [...current, tenant.name]
                      : current.filter((t) => t !== tenant.name)
                  )
                }
              />
              <label
                htmlFor={`invite-tenant-${tenant.id}`}
                className="flex-1 cursor-pointer truncate text-xs select-none"
              >
                {tenant.name}
              </label>
            </li>
          ))}
        </ul>
      </div>
      <div className="space-y-2 border-t pt-2">
        <Label>First password</Label>
        <div className="flex gap-4">
          <label className="flex cursor-pointer items-center gap-1.5 text-xs select-none">
            <input
              type="radio"
              name="password-mode"
              value="generate"
              checked={passwordMode === "generate"}
              onChange={() => setPasswordMode("generate")}
            />
            Generate
          </label>
          <label className="flex cursor-pointer items-center gap-1.5 text-xs select-none">
            <input
              type="radio"
              name="password-mode"
              value="set"
              checked={passwordMode === "set"}
              onChange={() => setPasswordMode("set")}
            />
            Set it now
          </label>
        </div>
        {passwordMode === "set" ? (
          <div className="space-y-1.5">
            <Input
              id="user-set-password"
              type="password"
              value={setPassword}
              onChange={(e) => setSetPassword(e.target.value)}
              placeholder="Enter password"
              aria-label="Password"
            />
            <p className="text-xs text-muted-foreground">
              At least {passwordRulesState.data?.minLength ?? 8} characters. Must not equal the
              username or email.
            </p>
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">
            A temporary password will be generated and shown once.
          </p>
        )}
        <div className="flex items-center gap-2 pt-1">
          <Checkbox
            id="must-change-password"
            checked={mustChangePassword}
            onCheckedChange={(c) => setMustChangePassword(c === true)}
            aria-label="Must change password at first sign-in"
          />
          <label
            htmlFor="must-change-password"
            className="cursor-pointer text-xs select-none"
          >
            Must change password at first sign-in
          </label>
        </div>
      </div>
    </FormPage>
  )
}
