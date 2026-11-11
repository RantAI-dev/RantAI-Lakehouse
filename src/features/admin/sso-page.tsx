"use client"

import * as React from "react"
import { useSearchParams } from "next/navigation"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import {
  identityService,
  settingsService,
  type RoleMapRow,
  type SsoSettingsInput,
  type SsoTestResult,
} from "@/services"
import { SettingField, SET_BY_SERVER } from "./setting-field"

/**
 * SSO / OIDC configuration page (`ADM-12`, plan D9).
 *
 * Inside Settings -> Sign-in:
 * - Issuer, client ID, client secret (with Replace/Clear toggle);
 * - Provider name, groups claim, JIT provisioning checkbox;
 * - Role map rows with a role Select from `identityService.listRoles`;
 * - Fields with `source === "server"` are read-only with "Set by the server";
 * - "Test sign-in" starts an OIDC flow in a new tab; on return it reads `ssoTest`
 *   from the URL and renders the test result card;
 * - Only an unrestricted Platform Admin (`canEdit`) may save or test SSO;
 *   otherwise all fields are read-only with a notice.
 */
export function SsoPage() {
  const { hasPermission } = useAuth()
  return <SsoPageForm canWrite={hasPermission("settings:write")} />
}

export function SsoPageForm({ canWrite }: { canWrite: boolean }) {
  const searchParams = useSearchParams()
  const ssoTestId = searchParams?.get("ssoTest") ?? null

  const state = useService(
    (signal) => (canWrite ? settingsService.getSso(signal) : Promise.reject(new Error("Forbidden"))),
    [canWrite]
  )
  const rolesState = useService(
    (signal) => (canWrite ? identityService.listRoles(signal) : Promise.resolve([])),
    [canWrite]
  )

  const save = useServiceAction(
    withNotify(
      { success: "SSO settings saved", error: "SSO settings were not saved" },
      (signal: AbortSignal, input: SsoSettingsInput) => settingsService.saveSso(input, signal)
    )
  )

  const testAction = useServiceAction((signal: AbortSignal, input: SsoSettingsInput) =>
    settingsService.startSsoTest(input, signal)
  )

  const loaded = state.status === "success" ? state.data : null
  const canEdit = Boolean(loaded?.canEdit)

  const [issuer, setIssuer] = React.useState("")
  const [clientId, setClientId] = React.useState("")
  const [secret, setSecret] = React.useState("")
  const [secretChoice, setSecretChoice] = React.useState<"keep" | "replace" | "clear">("keep")
  const [providerName, setProviderName] = React.useState("")
  const [groupsClaim, setGroupsClaim] = React.useState("")
  const [jitProvisioning, setJitProvisioning] = React.useState(false)
  const [roleMapRows, setRoleMapRows] = React.useState<RoleMapRow[]>([])

  const [testResult, setTestResult] = React.useState<SsoTestResult | null>(null)
  const [testResultError, setTestResultError] = React.useState<string | null>(null)
  const [testResultLoading, setTestResultLoading] = React.useState(false)
  const [dismissedTest, setDismissedTest] = React.useState(false)

  React.useEffect(() => {
    if (!loaded) return
    setIssuer(loaded.issuer.value ?? "")
    setClientId(loaded.clientId.value ?? "")
    setSecret("")
    setSecretChoice("keep")
    setProviderName(loaded.providerName.value ?? "")
    setGroupsClaim(loaded.groupsClaim.value ?? "")
    setJitProvisioning(loaded.jitProvisioning.value ?? false)
    setRoleMapRows(loaded.roleMap.value ?? [])
  }, [loaded])

  React.useEffect(() => {
    if (!ssoTestId) {
      setTestResult(null)
      setTestResultError(null)
      return
    }
    setDismissedTest(false)
    setTestResultLoading(true)
    setTestResultError(null)
    let cancelled = false

    settingsService
      .getSsoTestResult(ssoTestId)
      .then((res) => {
        if (!cancelled) {
          setTestResult(res)
          setTestResultLoading(false)
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setTestResultError(err instanceof Error ? err.message : "Failed to load test result")
          setTestResultLoading(false)
        }
      })

    return () => {
      cancelled = true
    }
  }, [ssoTestId])

  const rolesList = rolesState.status === "success" ? rolesState.data : []
  const rolesRecord = React.useMemo(() => {
    const record: Record<string, string> = {}
    for (const r of rolesList) {
      record[r.name] = r.name
    }
    for (const row of roleMapRows) {
      if (row.role && !record[row.role]) {
        record[row.role] = row.role
      }
    }
    return record
  }, [rolesList, roleMapRows])

  async function handleSave() {
    if (!loaded) return
    const input: SsoSettingsInput = {}
    if (loaded.issuer.source !== "server") {
      input.issuer = issuer.trim() === "" ? null : issuer.trim()
    }
    if (loaded.clientId.source !== "server") {
      input.clientId = clientId.trim() === "" ? null : clientId.trim()
    }
    if (loaded.providerName.source !== "server") {
      input.providerName = providerName.trim() === "" ? null : providerName.trim()
    }
    if (loaded.groupsClaim.source !== "server") {
      input.groupsClaim = groupsClaim.trim() === "" ? null : groupsClaim.trim()
    }
    if (loaded.jitProvisioning.source !== "server") {
      input.jitProvisioning = jitProvisioning
    }
    if (loaded.roleMap.source !== "server") {
      input.roleMap = roleMapRows
        .map((r) => ({ group: r.group.trim(), role: r.role.trim() }))
        .filter((r) => r.group !== "" && r.role !== "")
    }
    if (loaded.clientSecret.source !== "server") {
      if (secretChoice === "replace" && secret !== "") input.clientSecret = secret
      if (secretChoice === "clear") input.clientSecret = null
    }

    const result = await save.run(input)
    if (result) {
      setSecret("")
      setSecretChoice("keep")
      state.reload()
    }
  }

  async function handleTest() {
    if (!loaded) return
    const draft: SsoSettingsInput = {
      issuer: issuer.trim() === "" ? null : issuer.trim(),
      clientId: clientId.trim() === "" ? null : clientId.trim(),
      providerName: providerName.trim() === "" ? null : providerName.trim(),
      groupsClaim: groupsClaim.trim() === "" ? null : groupsClaim.trim(),
      jitProvisioning,
      roleMap: roleMapRows
        .map((r) => ({ group: r.group.trim(), role: r.role.trim() }))
        .filter((r) => r.group !== "" && r.role !== ""),
    }
    if (secretChoice === "replace" && secret !== "") {
      draft.clientSecret = secret
    }
    const res = await testAction.run(draft)
    if (res?.authorizeUrl) {
      window.open(res.authorizeUrl, "_blank")
    }
  }

  if (!canWrite) {
    return (
      <div className="flex flex-col gap-4">
        <PageHeader
          title="Sign-in"
          description="OIDC single sign-on for this deployment."
        />
        <p className="text-sm text-muted-foreground">
          Only an administrator can view or change single sign-on settings.
        </p>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Sign-in"
        description="OIDC single sign-on for this deployment."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}

      {ssoTestId && !dismissedTest ? (
        <div
          className="flex flex-col gap-2 rounded-lg border p-4 bg-muted/30"
          role="region"
          aria-label="Test sign-in result"
        >
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium">Test sign-in result</h3>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setDismissedTest(true)}
              aria-label="Dismiss test result"
            >
              Dismiss
            </Button>
          </div>
          {testResultLoading ? (
            <p className="text-sm text-muted-foreground">Loading test result...</p>
          ) : null}
          {testResultError ? (
            <p role="alert" className="text-sm text-destructive">
              {testResultError}
            </p>
          ) : null}
          {testResult ? (
            testResult.error || testResult.ok === false ? (
              <p role="alert" className="text-sm text-destructive">
                {testResult.error || "The test sign-in failed."}
              </p>
            ) : (
              <div className="flex flex-col gap-1 text-sm">
                <p>
                  Subject: <span className="font-mono font-medium">{testResult.subject ?? "None"}</span>
                </p>
                <p>
                  Email: <span className="font-mono font-medium">{testResult.email ?? "None"}</span>
                </p>
                <p>
                  Groups:{" "}
                  <span className="font-mono font-medium">
                    {testResult.groups && testResult.groups.length > 0
                      ? testResult.groups.join(", ")
                      : "None"}
                  </span>
                </p>
                <p>
                  Mapped roles:{" "}
                  <span className="font-mono font-medium">
                    {testResult.mappedRoles && testResult.mappedRoles.length > 0
                      ? testResult.mappedRoles.join(", ")
                      : "None"}
                  </span>
                </p>
              </div>
            )
          ) : null}
        </div>
      ) : null}

      {loaded && !loaded.ready ? (
        <p
          role="status"
          className="max-w-sm rounded-lg border border-amber-300 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-200"
        >
          Sign-in is not ready.{" "}
          {loaded.notReadyReason ?? "The saved connection cannot be used right now."}
        </p>
      ) : null}

      {loaded ? (
        <div className="flex max-w-sm flex-col gap-4">
          <SettingField label="Issuer URL" source={loaded.issuer.source} id="sso-issuer">
            <Input
              id="sso-issuer"
              aria-label="Issuer URL"
              value={issuer}
              onChange={(e) => setIssuer(e.target.value)}
              placeholder="https://auth.example.com"
              disabled={!canEdit || loaded.issuer.source === "server"}
              autoComplete="off"
            />
          </SettingField>

          <SettingField label="Client ID" source={loaded.clientId.source} id="sso-client-id">
            <Input
              id="sso-client-id"
              aria-label="Client ID"
              value={clientId}
              onChange={(e) => setClientId(e.target.value)}
              placeholder="client-id"
              disabled={!canEdit || loaded.clientId.source === "server"}
              autoComplete="off"
            />
          </SettingField>

          <SettingField
            label="Client secret"
            source={loaded.clientSecret.source}
            id="sso-client-secret"
          >
            {loaded.clientSecret.source === "server" ? null : secretChoice === "replace" ? (
              <>
                <Input
                  id="sso-client-secret"
                  aria-label="Client secret"
                  type="password"
                  value={secret}
                  onChange={(e) => setSecret(e.target.value)}
                  disabled={!canEdit}
                  autoComplete="new-password"
                />
                <Button
                  variant="ghost"
                  size="sm"
                  className="w-fit"
                  onClick={() => {
                    setSecret("")
                    setSecretChoice("keep")
                  }}
                  disabled={!canEdit}
                >
                  Keep the saved secret
                </Button>
              </>
            ) : (
              <>
                <p className="text-sm">{loaded.clientSecret.set ? "Set" : "Not set"}</p>
                {secretChoice === "clear" ? (
                  <p className="text-xs text-amber-700 dark:text-amber-400">
                    The saved secret is cleared when you save.
                  </p>
                ) : null}
                {canEdit ? (
                  <div className="flex gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => setSecretChoice("replace")}
                    >
                      Replace
                    </Button>
                    {loaded.clientSecret.set ? (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setSecretChoice("clear")}
                      >
                        Clear
                      </Button>
                    ) : null}
                  </div>
                ) : null}
              </>
            )}
          </SettingField>

          <SettingField
            label="Provider display name"
            source={loaded.providerName.source}
            id="sso-provider-name"
          >
            <Input
              id="sso-provider-name"
              aria-label="Provider display name"
              value={providerName}
              onChange={(e) => setProviderName(e.target.value)}
              placeholder="default"
              disabled={!canEdit || loaded.providerName.source === "server"}
              autoComplete="off"
            />
          </SettingField>

          <SettingField
            label="Groups claim"
            source={loaded.groupsClaim.source}
            id="sso-groups-claim"
          >
            <Input
              id="sso-groups-claim"
              aria-label="Groups claim"
              value={groupsClaim}
              onChange={(e) => setGroupsClaim(e.target.value)}
              placeholder="groups"
              disabled={!canEdit || loaded.groupsClaim.source === "server"}
              autoComplete="off"
            />
          </SettingField>

          <SettingField
            label="Just-in-time provisioning"
            source={loaded.jitProvisioning.source}
            id="sso-jit-provisioning"
          >
            <div className="flex items-center gap-2">
              <Checkbox
                id="sso-jit-provisioning"
                aria-label="Just-in-time provisioning"
                checked={jitProvisioning}
                onCheckedChange={(checked) => setJitProvisioning(Boolean(checked))}
                disabled={!canEdit || loaded.jitProvisioning.source === "server"}
              />
              <Label htmlFor="sso-jit-provisioning" className="text-xs font-normal text-muted-foreground">
                Create accounts for new SSO users automatically.
              </Label>
            </div>
          </SettingField>

          <div className="flex flex-col gap-2" role="group" aria-labelledby="sso-role-map-label">
            <Label id="sso-role-map-label">Role mapping</Label>
            <p className="text-xs text-muted-foreground">
              Map IdP group names to deployment roles.
            </p>
            {loaded.roleMap.source === "server" ? (
              <p className="text-xs text-muted-foreground">{SET_BY_SERVER}</p>
            ) : null}
            {roleMapRows.map((row, i) => (
              <div key={i} className="flex items-center gap-2">
                <Input
                  className="flex-1"
                  aria-label={`IdP group row ${i + 1}`}
                  placeholder="IdP group"
                  value={row.group}
                  onChange={(e) => {
                    const next = [...roleMapRows]
                    next[i] = { ...next[i], group: e.target.value }
                    setRoleMapRows(next)
                  }}
                  disabled={!canEdit || loaded.roleMap.source === "server"}
                />
                <Select
                  value={row.role}
                  items={rolesRecord}
                  onValueChange={(v) => {
                    const next = [...roleMapRows]
                    next[i] = { ...next[i], role: v ?? "" }
                    setRoleMapRows(next)
                  }}
                  disabled={!canEdit || loaded.roleMap.source === "server"}
                >
                  <SelectTrigger aria-label={`Role row ${i + 1}`} className="w-36">
                    <SelectValue placeholder="Role" />
                  </SelectTrigger>
                  <SelectContent>
                    {rolesList.map((r) => (
                      <SelectItem key={r.name} value={r.name}>
                        {r.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {canEdit && loaded.roleMap.source !== "server" ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-label={`Remove role mapping row ${i + 1}`}
                    onClick={() => {
                      setRoleMapRows(roleMapRows.filter((_, idx) => idx !== i))
                    }}
                  >
                    Remove
                  </Button>
                ) : null}
              </div>
            ))}
            {canEdit && loaded.roleMap.source !== "server" ? (
              <Button
                variant="outline"
                size="sm"
                className="w-fit"
                onClick={() => {
                  const defaultRole = rolesList[0]?.name ?? "viewer"
                  setRoleMapRows([...roleMapRows, { group: "", role: defaultRole }])
                }}
              >
                Add role mapping
              </Button>
            ) : null}
          </div>

          {canEdit ? (
            <div className="flex items-center gap-3">
              <Button
                className="w-fit"
                onClick={handleSave}
                disabled={save.status === "pending"}
              >
                Save
              </Button>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">
              Only a Platform Admin can change single sign-on settings.
            </p>
          )}

          <section className="flex flex-col gap-2" aria-labelledby="sso-test">
            <h2 id="sso-test" className="text-sm font-medium">
              Test sign-in
            </h2>
            <p className="text-xs text-muted-foreground">
              Opens the IdP sign-in flow in a new window with the current settings, without creating a session or user.
            </p>
            <div className="flex items-center gap-3">
              <Button
                variant="outline"
                className="w-fit"
                onClick={handleTest}
                disabled={!canEdit || testAction.status === "pending"}
              >
                Test sign-in
              </Button>
            </div>
          </section>
        </div>
      ) : null}
    </div>
  )
}