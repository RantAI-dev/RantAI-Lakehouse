"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { FormPage } from "@/components/patterns/form-page"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { identityService } from "@/services"
import { PermissionPicker } from "./permission-picker"

/**
 * `/admin/service-identities/new` (`WEB-7` D2): the create form, moved out
 * of the `CreateSheet` side panel onto its own page, with every field it
 * had.
 *
 * `DATA-5` D10 keeps its hint: an admin who grants `iceberg:read` needs the
 * address the outside tool points at, and the fact that the key comes from
 * Rotate, not from this form.
 */
export function ServiceIdentityCreatePage() {
  const router = useRouter()
  const backHref = "/admin/service-identities"
  const [name, setName] = React.useState("")
  const [scopes, setScopes] = React.useState("")
  const [environment, setEnvironment] = React.useState("")
  /**
   * The console address an outside tool points at (`DATA-5` D10, ADR 0016).
   * Read in an effect so the server-rendered pass never touches `window`
   * and the two renders cannot disagree.
   */
  const [consoleOrigin, setConsoleOrigin] = React.useState("")
  React.useEffect(() => {
    setConsoleOrigin(window.location.origin)
  }, [])
  const create = useServiceAction(
    withNotify(
      {
        success: "Service identity created",
        error: "Failed to create service identity",
      },
      (signal, input: Parameters<typeof identityService.createServiceIdentity>[0]) =>
        identityService.createServiceIdentity(input, signal)
    )
  )

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      scopes: scopes
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
      environment: environment.trim(),
    })
    if (result) router.push(backHref)
  }

  return (
    <FormPage
      title="Create Service Identity"
      description="Issue a new non-human service identity."
      backHref={backHref}
      backLabel="Back to Service Identities"
      submitLabel="Create"
      submittingLabel="Creating…"
      canSubmit={Boolean(name.trim() && scopes.trim() && environment.trim())}
      submitting={create.status === "pending"}
      error={create.status === "error" ? create.error.message : null}
      onSubmit={() => void handleCreate()}
      onCancel={() => router.push(backHref)}
    >
      <div className="space-y-1.5">
        <Label htmlFor="si-name">Name</Label>
        <Input id="si-name" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="si-scopes">Scopes</Label>
        <PermissionPicker id="si-scopes" value={scopes} onChange={setScopes} />
        <p className="text-xs text-muted-foreground">
          <code>iceberg:read</code> lets an outside tool read published Gold tables at{" "}
          <code data-testid="outside-reader-address">{consoleOrigin}/api/iceberg</code>. Use Rotate
          to get its key.
        </p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="si-env">Environment</Label>
        <Input
          id="si-env"
          value={environment}
          onChange={(e) => setEnvironment(e.target.value)}
          placeholder="production"
        />
      </div>
    </FormPage>
  )
}
