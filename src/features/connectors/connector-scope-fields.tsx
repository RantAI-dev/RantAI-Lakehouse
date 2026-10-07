"use client"

import * as React from "react"
import { Building2Icon, CheckIcon, LayersIcon, MapPinIcon, UsersIcon } from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Input } from "@/components/ui/input"
import { useAuth } from "@/features/auth/auth-provider"
import { cn } from "@/lib/utils"
import { StepSectionHeading } from "./connector-form-parts"

/** The tenant cards' grid. The environment and residency fields sit in its
 * first column, so all three controls in the step are the same width. */
const SCOPE_GRID = "grid gap-2 sm:grid-cols-2 xl:grid-cols-3"

/**
 * The Tenant and residency step, shared by the create and edit pages: a
 * titled section each for the tenant, the environment and the residency,
 * stacked like the Connection step's sections.
 */
export function ScopeFields({
  tenantDescription,
  tenantId,
  onTenantChange,
  environment,
  onEnvironmentChange,
  residency,
  onResidencyChange,
}: {
  tenantDescription: string
  tenantId: string
  onTenantChange: (tenantId: string) => void
  environment: string
  onEnvironmentChange: (value: string) => void
  residency: string
  onResidencyChange: (value: string) => void
}) {
  return (
    <div className="space-y-6">
      <section className="space-y-3">
        <StepSectionHeading icon={<UsersIcon className="size-4" />} title="Tenant" description={tenantDescription} />
        <TenantPicker value={tenantId} onChange={onTenantChange} />
      </section>
      <section className="space-y-3 border-t border-border pt-5">
        <StepSectionHeading
          icon={<LayersIcon className="size-4" />}
          title="Environment"
          description="Which stage of the source system this is. Shown and filterable in the connector list."
        />
        <div className={SCOPE_GRID}>
          <EnvironmentInput id="connector-environment" value={environment} onChange={onEnvironmentChange} />
        </div>
      </section>
      <section className="space-y-3 border-t border-border pt-5">
        <StepSectionHeading
          icon={<MapPinIcon className="size-4" />}
          title="Residency"
          description="Where the source's data is kept, recorded with the connector for governance."
        />
        <div className={SCOPE_GRID}>
          <Input
            id="connector-residency"
            aria-label="Residency"
            value={residency}
            onChange={(e) => onResidencyChange(e.target.value)}
            placeholder="e.g. in-region"
            autoComplete="off"
          />
        </div>
      </section>
    </div>
  )
}

/**
 * The tenant a new connector belongs to, as one card per tenant the user
 * belongs to. A connector's tenant decides who sees it in the connector
 * list, so the one the user is acting as right now is marked.
 */
export function TenantPicker({ value, onChange }: { value: string; onChange: (tenantId: string) => void }) {
  const { user, activeTenantId } = useAuth()
  const tenants = user?.tenants ?? []
  if (tenants.length === 0) {
    return (
      <p className="rounded-lg border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
        Your account belongs to no tenant, so a connector you create would not appear in any connector list. Ask
        an administrator to add you to a tenant.
      </p>
    )
  }
  return (
    <div role="radiogroup" aria-label="Tenant" className={SCOPE_GRID}>
      {tenants.map((tenant) => {
        const checked = tenant.id === value
        return (
          <button
            key={tenant.id}
            type="button"
            role="radio"
            aria-checked={checked}
            onClick={() => onChange(tenant.id)}
            className={cn(
              "relative flex items-center gap-3 rounded-lg border px-3 py-2.5 text-left transition-colors outline-none",
              "focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50",
              checked ? "border-primary bg-primary/5 ring-1 ring-primary" : "border-border hover:bg-muted/40"
            )}
          >
            <span
              className={cn(
                "flex size-8 shrink-0 items-center justify-center rounded-md",
                checked ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground"
              )}
            >
              <Building2Icon className="size-4" />
            </span>
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5">
                <span className="truncate text-sm font-medium">{tenant.name}</span>
                {tenant.id === activeTenantId ? (
                  <Badge variant="outline" className="h-4 px-1.5 text-[10px]">
                    Current
                  </Badge>
                ) : null}
              </span>
              <span className="block truncate font-mono text-xs text-muted-foreground">{tenant.slug}</span>
            </span>
            {checked ? <CheckIcon className="absolute top-2 right-2 size-3.5 text-primary" aria-hidden /> : null}
          </button>
        )
      })}
    </div>
  )
}

const ENVIRONMENT_PRESETS = ["production", "staging", "development"]

/**
 * Free text, as the server stores it, with the common values one click
 * away so connectors in the same environment are spelled the same way.
 */
export function EnvironmentInput({
  id,
  value,
  onChange,
}: {
  id: string
  value: string
  onChange: (value: string) => void
}) {
  return (
    <div className="space-y-2">
      <div role="group" aria-label="Common environments" className="flex flex-wrap gap-1.5">
        {ENVIRONMENT_PRESETS.map((preset) => {
          const active = value.trim() === preset
          return (
            <button
              key={preset}
              type="button"
              aria-pressed={active}
              onClick={() => onChange(preset)}
              className={cn(
                "rounded-full border px-2.5 py-0.5 text-xs font-medium transition-colors outline-none",
                "focus-visible:ring-[3px] focus-visible:ring-ring/50",
                active
                  ? "border-primary bg-primary/10 text-primary"
                  : "border-border text-muted-foreground hover:bg-muted/60 hover:text-foreground"
              )}
            >
              {preset}
            </button>
          )
        })}
      </div>
      <Input
        id={id}
        aria-label="Environment"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="production"
        autoComplete="off"
      />
    </div>
  )
}
