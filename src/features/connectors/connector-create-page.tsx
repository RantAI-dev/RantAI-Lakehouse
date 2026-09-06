"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { FormReviewSummary } from "@/components/patterns/form-review-summary"
import { FormStepLayout, type FormStep } from "@/components/patterns/form-step-layout"
import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { cn } from "@/lib/utils"
import { connectorService } from "@/services"
import type { Connector } from "@/services/contracts/connectors"

const STEPS: FormStep[] = [
  { id: "type", label: "Type", description: "Connector kind" },
  { id: "connection", label: "Connection", description: "Host and secret" },
  { id: "test", label: "Test", description: "Validate connectivity" },
  { id: "discover", label: "Discover", description: "Capabilities" },
  { id: "scope", label: "Scope", description: "Tenant and residency" },
  { id: "review", label: "Review", description: "Confirm" },
]

const TYPES = ["PostgreSQL CDC", "Kafka", "Object storage", "SaaS REST", "JDBC"]
const CAPABILITIES = ["CDC", "schema discovery", "checkpoint", "consume", "produce", "list", "read"]

export function ConnectorCreatePage() {
  const router = useRouter()
  const [step, setStep] = React.useState(0)
  const [name, setName] = React.useState("")
  const [type, setType] = React.useState(TYPES[0])
  const [direction, setDirection] = React.useState<Connector["direction"]>("source")
  const [host, setHost] = React.useState("")
  const [secretRef, setSecretRef] = React.useState("vault://connectors/")
  const [tested, setTested] = React.useState(false)
  const [capabilities, setCapabilities] = React.useState<string[]>(["schema discovery"])
  const [environment, setEnvironment] = React.useState("production")
  const [tenant, setTenant] = React.useState("Nusantara Finance")
  const [residency, setResidency] = React.useState("Jakarta (ID)")
  const create = useServiceAction(
    withNotify(
      { success: "Connector created", error: "Failed to create connector" },
      (signal, input: Parameters<typeof connectorService.createConnector>[0]) =>
        connectorService.createConnector(input, signal)
    )
  )

  const canProceed =
    (step === 0 && Boolean(name.trim() && type)) ||
    (step === 1 && Boolean(host.trim() && secretRef.trim())) ||
    (step === 2 && tested) ||
    (step === 3 && capabilities.length > 0) ||
    (step === 4 && Boolean(environment.trim() && tenant.trim() && residency.trim())) ||
    step === 5

  async function handleSubmit() {
    const result = await create.run({
      name: name.trim(),
      type,
      direction,
      host: host.trim(),
      secretRef: secretRef.trim(),
      environment: environment.trim(),
      tenant: tenant.trim(),
      residency: residency.trim(),
      capabilities,
    })
    if (result) router.push("/connectors")
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="New Connector"
        description="Configure a source or sink with test, discovery, and residency scope."
        actions={
          <Button variant="outline" size="sm" render={<Link href="/connectors" />}>
            Cancel
          </Button>
        }
      />
      <FormStepLayout
        steps={STEPS}
        currentIndex={step}
        onStepChange={setStep}
        canProceed={canProceed}
        onSubmit={handleSubmit}
        submitLabel="Create connector"
        submitting={create.status === "pending"}
      >
        {step === 0 ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Name" className="sm:col-span-2">
              <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="postgres core CDC" />
            </Field>
            <Field label="Type">
              <select
                className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                value={type}
                onChange={(e) => setType(e.target.value)}
              >
                {TYPES.map((t) => (
                  <option key={t} value={t}>{t}</option>
                ))}
              </select>
            </Field>
            <Field label="Direction">
              <select
                className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                value={direction}
                onChange={(e) => setDirection(e.target.value as Connector["direction"])}
              >
                <option value="source">Source</option>
                <option value="sink">Sink</option>
                <option value="bidirectional">Bidirectional</option>
              </select>
            </Field>
          </div>
        ) : null}
        {step === 1 ? (
          <div className="grid gap-3">
            <Field label="Host / endpoint">
              <Input value={host} onChange={(e) => setHost(e.target.value)} placeholder="db.internal:5432" />
            </Field>
            <Field label="Secret reference">
              <Input
                value={secretRef}
                onChange={(e) => setSecretRef(e.target.value)}
                placeholder="vault://connectors/pg-core"
              />
            </Field>
            <p className="text-xs text-muted-foreground">
              Secrets are referenced by path only; values are never stored in the browser.
            </p>
          </div>
        ) : null}
        {step === 2 ? (
          <div className="space-y-3">
            <p className="text-sm text-muted-foreground">
              Run a mock connectivity and authentication check against {host || "the endpoint"}.
            </p>
            <Button
              type="button"
              size="sm"
              variant={tested ? "secondary" : "default"}
              onClick={() => setTested(true)}
            >
              {tested ? "Test passed" : "Test connection"}
            </Button>
          </div>
        ) : null}
        {step === 3 ? (
          <div className="space-y-2">
            <p className="text-sm font-medium">Capabilities</p>
            <div className="flex flex-wrap gap-2">
              {CAPABILITIES.map((cap) => {
                const active = capabilities.includes(cap)
                return (
                  <button
                    key={cap}
                    type="button"
                    onClick={() =>
                      setCapabilities((prev) =>
                        active ? prev.filter((c) => c !== cap) : [...prev, cap]
                      )
                    }
                    className={cn(
                      "rounded-full border px-3 py-1 text-xs font-medium",
                      active
                        ? "border-primary bg-primary/15 text-primary"
                        : "border-border text-muted-foreground"
                    )}
                  >
                    {cap}
                  </button>
                )
              })}
            </div>
          </div>
        ) : null}
        {step === 4 ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Environment">
              <Input value={environment} onChange={(e) => setEnvironment(e.target.value)} />
            </Field>
            <Field label="Tenant">
              <Input value={tenant} onChange={(e) => setTenant(e.target.value)} />
            </Field>
            <Field label="Residency" className="sm:col-span-2">
              <Input value={residency} onChange={(e) => setResidency(e.target.value)} />
            </Field>
          </div>
        ) : null}
        {step === 5 ? (
          <FormReviewSummary
            sections={[
              {
                title: "Connector",
                items: [
                  { label: "Name", value: name },
                  { label: "Type", value: type },
                  { label: "Direction", value: direction },
                  { label: "Host", value: host },
                  { label: "Secret", value: secretRef },
                ],
              },
              {
                title: "Scope",
                items: [
                  { label: "Environment", value: environment },
                  { label: "Tenant", value: tenant },
                  { label: "Residency", value: residency },
                  { label: "Capabilities", value: capabilities.join(", ") },
                ],
              },
            ]}
          />
        ) : null}
        {create.status === "error" ? (
          <p className="text-sm text-destructive">{create.error.message}</p>
        ) : null}
      </FormStepLayout>
    </div>
  )
}

function Field({
  label,
  children,
  className,
}: {
  label: string
  children: React.ReactNode
  className?: string
}) {
  return (
    <div className={cn("space-y-1.5", className)}>
      <Label>{label}</Label>
      {children}
    </div>
  )
}
