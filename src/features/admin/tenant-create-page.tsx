"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { FormPage } from "@/components/patterns/form-page"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { identityService } from "@/services"

/**
 * `/admin/tenants/new` (`WEB-7` D2): the create form, moved out of the
 * `CreateSheet` side panel onto its own page, with every field it had.
 */
export function TenantCreatePage() {
  const router = useRouter()
  const backHref = "/admin/tenants"
  const [name, setName] = React.useState("")
  const [slug, setSlug] = React.useState("")
  const [plan, setPlan] = React.useState("")
  const [residency, setResidency] = React.useState("")
  const create = useServiceAction(
    withNotify(
      { success: "Tenant created", error: "Failed to create tenant" },
      (signal, input: Parameters<typeof identityService.createTenant>[0]) =>
        identityService.createTenant(input, signal)
    )
  )

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      slug: slug.trim(),
      plan: plan.trim(),
      residency: residency.trim(),
    })
    if (result) router.push(backHref)
  }

  return (
    <FormPage
      title="Create Tenant"
      description="Provision a tenant with residency and quota."
      backHref={backHref}
      backLabel="Back to Tenants"
      submitLabel="Create"
      submittingLabel="Creating…"
      canSubmit={Boolean(name.trim() && slug.trim() && plan.trim() && residency.trim())}
      submitting={create.status === "pending"}
      error={create.status === "error" ? create.error.message : null}
      onSubmit={() => void handleCreate()}
      onCancel={() => router.push(backHref)}
    >
      <div className="space-y-1.5">
        <Label htmlFor="tenant-name">Name</Label>
        <Input id="tenant-name" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="tenant-slug">Slug</Label>
        <Input
          id="tenant-slug"
          value={slug}
          onChange={(e) => setSlug(e.target.value)}
          placeholder="acme"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="tenant-plan">Plan</Label>
        <Input
          id="tenant-plan"
          value={plan}
          onChange={(e) => setPlan(e.target.value)}
          placeholder="Enterprise"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="tenant-residency">Residency</Label>
        <Input
          id="tenant-residency"
          value={residency}
          onChange={(e) => setResidency(e.target.value)}
          placeholder="ap-southeast-1"
        />
      </div>
    </FormPage>
  )
}
