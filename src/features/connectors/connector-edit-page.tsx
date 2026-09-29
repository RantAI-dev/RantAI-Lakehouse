"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { FormReviewSummary } from "@/components/patterns/form-review-summary"
import { FormStepLayout, type FormStep } from "@/components/patterns/form-step-layout"
import { PageHeader } from "@/components/patterns/page-header"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useAuth } from "@/features/auth/auth-provider"
import { useService } from "@/hooks/use-service"
import { connectorService } from "@/services"
import { isServiceError } from "@/services/errors"
import type {
  Connector,
  ConnectorDetail,
  ConnectorTestResult,
  ConnectorType,
  CredentialKind,
  IngestSpec,
  SetConnectorCredentialRequest,
  UpdateConnectorInput,
} from "@/services/contracts/connectors"
import {
  CredentialFields,
  type CredentialSlots,
  DialFormFor,
  Field,
  TenantField,
  credentialProblems,
  credentialSlotsFor,
  credentialSlotsKey,
  hostFromDial,
  normalizedCredential,
  usesNoCredential as needsNoCredential,
} from "./connector-form-parts"
import { ConnectorDeleteDialog } from "./connector-delete-dialog"

const STEPS: FormStep[] = [
  { id: "type", label: "Type", description: "Name and direction" },
  { id: "connection", label: "Connection", description: "Dial and secret" },
  { id: "scope", label: "Tenant and residency" },
  { id: "review", label: "Review", description: "Confirm changes" },
]

type Loaded = { detail: ConnectorDetail; spec: IngestSpec; type: ConnectorType | null }

/** One part of a save, reported back to the user in order. */
type SaveStep = { label: string; status: "done" | "failed"; message?: string }

/**
 * Edit an existing connector with the same steps as creating one. A
 * connector's parts are owned by different routes, so a save is a short
 * sequence — basic fields (`PATCH`), connection settings (ingest spec),
 * tenant, credential — followed by a connection test. It stops at the first
 * failure and reports exactly which parts were saved, rather than a bare
 * "failed".
 *
 * The type is shown but not editable: it fixes the adapter, the shape of the
 * connection settings and the credential names, so a different type is a
 * different connector.
 */
export function ConnectorEditPage({ connectorId }: { connectorId: string }) {
  const loaded = useService<Loaded>(
    async (signal) => {
      const [detail, spec, types] = await Promise.all([
        connectorService.getConnector(connectorId, signal),
        connectorService.getIngestSpec(connectorId, signal),
        connectorService.listTypes(signal),
      ])
      return { detail, spec, type: types.find((t) => t.name === detail.type) ?? null }
    },
    [connectorId]
  )

  if (loaded.status === "loading") return <LoadingSkeleton rows={6} />
  if (loaded.status === "error") return <ErrorState error={loaded.error} onRetry={loaded.reload} />
  return <EditForm key={loaded.data.detail.id} loaded={loaded.data} onReload={loaded.reload} />
}

function EditForm({ loaded, onReload }: { loaded: Loaded; onReload: () => void }) {
  const { detail, spec, type } = loaded
  const { user } = useAuth()
  const adapter = spec.adapter ?? type?.adapter ?? null
  const originalDial = React.useMemo(() => (spec.dial ?? {}) as Record<string, unknown>, [spec.dial])

  const [step, setStep] = React.useState(0)
  const [name, setName] = React.useState(detail.name)
  const [direction, setDirection] = React.useState<Connector["direction"]>(detail.direction)
  const [dial, setDial] = React.useState<Record<string, unknown> | null>(originalDial)
  // The fields follow the connection's auth type, the same way the create
  // page's do (an SFTP private key, a REST username + password, ...), so a
  // replacement is always the shape the source reads. While that shape
  // still has as many slots as the stored credential, the replacement keeps
  // the stored kinds (read from the reference names server-side): same
  // reference names, nothing to swap.
  const slots = withStoredKinds(credentialSlotsFor(adapter, dial), detail.credentialKind, detail.credentialSecondaryKind)
  const slotsKey = credentialSlotsKey(slots)
  // Write-only: sent once on save, cleared right after, never rendered back.
  const [primaryValue, setPrimaryValue] = React.useState("")
  const [secondaryValue, setSecondaryValue] = React.useState("")
  React.useEffect(() => {
    setPrimaryValue("")
    setSecondaryValue("")
  }, [slotsKey])
  const [environment, setEnvironment] = React.useState(detail.environment)
  const [tenantId, setTenantId] = React.useState(detail.tenantId ?? "")
  const [residency, setResidency] = React.useState(detail.residency)

  const [saving, setSaving] = React.useState(false)
  const [saveSteps, setSaveSteps] = React.useState<SaveStep[] | null>(null)
  const [testResult, setTestResult] = React.useState<ConnectorTestResult | null>(null)

  const noCredential = needsNoCredential(adapter, dial)
  const problems = credentialProblems({
    noCredential,
    optional: true,
    slots,
    primaryValue,
    secondaryValue,
  })

  // ── What changed ──────────────────────────────────────────────────────
  const basic: UpdateConnectorInput = {}
  if (name.trim() !== detail.name) basic.name = name.trim()
  if (direction !== detail.direction) basic.direction = direction
  if (environment.trim() !== detail.environment) basic.environment = environment.trim()
  if (residency.trim() !== detail.residency) basic.residency = residency.trim()
  const dialChanged = JSON.stringify(dial ?? {}) !== JSON.stringify(originalDial)
  if (dialChanged) {
    const host = hostFromDial(adapter, dial)
    if (host) basic.host = host
  }
  const basicChanged = Object.keys(basic).length > 0
  const tenantChanged = tenantId !== "" && tenantId !== (detail.tenantId ?? "")
  const credentialTyped = !noCredential && (primaryValue !== "" || secondaryValue !== "")
  const anyChange = basicChanged || dialChanged || tenantChanged || credentialTyped

  const tenantNameOf = (id: string | null) => user?.tenants.find((t) => t.id === id)?.name ?? (id ? "Other tenant" : "Unassigned")

  const canProceed =
    (step === 0 && Boolean(name.trim())) ||
    (step === 1 && problems.primary === null && problems.secondary === null) ||
    (step === 2 && Boolean(environment.trim() && residency.trim())) ||
    (step === 3 && anyChange)

  async function handleSave() {
    if (!adapter) return
    setSaving(true)
    setTestResult(null)
    const steps: SaveStep[] = []
    const record = (label: string, run: () => Promise<unknown>) => async () => {
      try {
        await run()
        steps.push({ label, status: "done" })
        return true
      } catch (error) {
        const denied = isServiceError(error) && error.status === 403
        steps.push({
          label,
          status: "failed",
          message: denied
            ? "You do not have permission for this change."
            : error instanceof Error
              ? error.message
              : String(error),
        })
        return false
      }
    }
    const credential: SetConnectorCredentialRequest = {}
    if (credentialTyped) {
      credential.primary = { kind: slots.primary.kind, value: normalizedCredential(slots.primary, primaryValue) }
      if (slots.secondary) {
        credential.secondary = {
          kind: slots.secondary.kind,
          value: normalizedCredential(slots.secondary, secondaryValue),
        }
      }
    }
    // Never keep a credential in memory longer than the request needs it.
    setPrimaryValue("")
    setSecondaryValue("")

    // Order matters: the connection settings are saved BEFORE the credential,
    // because the credential route tests the new value against the source
    // using the connector's SAVED connection settings.
    const sequence: (() => Promise<boolean>)[] = []
    if (basicChanged) {
      sequence.push(record("Name, direction, environment and residency", () =>
        connectorService.updateConnector(detail.id, basic)
      ))
    }
    if (dialChanged) {
      sequence.push(record("Connection settings", () =>
        connectorService.setIngestSpec(detail.id, {
          adapter: adapter as NonNullable<IngestSpec["adapter"]>,
          ingestMode: spec.ingestMode ?? (adapter === "cdc" ? "cdc" : "batch"),
          dial: (dial ?? {}) as IngestSpec["dial"],
          // Kept as they are: this page does not edit what is ingested.
          sourceObjects: spec.sourceObjects,
          ...(spec.scheduleCron ? { scheduleCron: spec.scheduleCron } : {}),
        })
      ))
    }
    if (tenantChanged) {
      sequence.push(record("Tenant", () => connectorService.assignTenant(detail.id, tenantId)))
    }
    if (credentialTyped) {
      sequence.push(record("Credential (tested against the source first)", () =>
        connectorService.setCredential(detail.id, credential)
      ))
    }

    let allSaved = true
    for (const run of sequence) {
      if (!(await run())) {
        allSaved = false
        break
      }
    }
    setSaveSteps([...steps])
    if (allSaved) {
      try {
        setTestResult(await connectorService.testConnection(detail.id))
      } catch {
        setTestResult(null)
      }
    }
    setSaving(false)
  }

  if (!adapter) {
    return (
      <div className="flex flex-col gap-4">
        <EditHeader />
        <p className="text-sm text-muted-foreground">
          This connector&apos;s type ({detail.type}) has no connection form in this build, so it cannot be edited here.
        </p>
        <DeleteSection detail={detail} />
      </div>
    )
  }

  if (saveSteps) {
    const failed = saveSteps.some((s) => s.status === "failed")
    return (
      <div className="flex flex-col gap-4">
        <EditHeader />
        <SectionCard
          title={failed ? "Some changes were not saved" : "Changes saved"}
          description={failed ? "Saving stopped at the first failure. Parts marked saved are in effect." : undefined}
        >
          <ul className="space-y-1.5 text-sm">
            {saveSteps.map((s) => (
              <li key={s.label} className={s.status === "done" ? "text-emerald-600 dark:text-emerald-400" : "text-destructive"}>
                {s.status === "done" ? "Saved" : "Not saved"} · {s.label}
                {s.message ? ` — ${s.message}` : ""}
              </li>
            ))}
          </ul>
          {testResult ? (
            <p
              className={
                !testResult.supported
                  ? "mt-3 text-sm text-muted-foreground"
                  : testResult.ok
                    ? "mt-3 text-sm text-emerald-600 dark:text-emerald-400"
                    : "mt-3 text-sm text-destructive"
              }
            >
              {!testResult.supported
                ? `This connector type cannot be tested yet · ${testResult.message}`
                : testResult.ok
                  ? `Connection test passed · ${testResult.message}`
                  : `Connection test failed · ${testResult.message}`}
            </p>
          ) : null}
          <div className="mt-3 flex flex-wrap gap-2">
            <Button size="sm" render={<Link href="/connectors" />}>
              Back to connectors
            </Button>
            <Button size="sm" variant="outline" onClick={onReload}>
              Edit again
            </Button>
          </div>
        </SectionCard>
      </div>
    )
  }

  const reviewItems: { label: string; value: string }[] = []
  if (basic.name) reviewItems.push({ label: "Name", value: `${detail.name} → ${basic.name}` })
  if (basic.direction) reviewItems.push({ label: "Direction", value: `${detail.direction} → ${basic.direction}` })
  if (basic.environment) reviewItems.push({ label: "Environment", value: `${detail.environment} → ${basic.environment}` })
  if (basic.residency !== undefined) reviewItems.push({ label: "Residency", value: `${detail.residency || "—"} → ${basic.residency}` })
  if (dialChanged) reviewItems.push({ label: "Connection settings", value: "Changed" })
  if (tenantChanged) {
    reviewItems.push({ label: "Tenant", value: `${tenantNameOf(detail.tenantId)} → ${tenantNameOf(tenantId)}` })
  }
  if (credentialTyped) {
    reviewItems.push({
      label: "Credential",
      value: `New ${slots.primary.label.toLowerCase()}${
        slots.secondary ? ` + ${slots.secondary.label.toLowerCase()}` : ""
      } · tested before it replaces the current one`,
    })
  }

  return (
    <div className="flex flex-col gap-4">
      <EditHeader />
      <FormStepLayout
        steps={STEPS}
        currentIndex={step}
        onStepChange={setStep}
        canProceed={canProceed}
        onSubmit={handleSave}
        submitLabel="Save changes"
        submitting={saving}
      >
        {step === 0 ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Name" htmlFor="connector-name" className="sm:col-span-2">
              <Input id="connector-name" value={name} onChange={(e) => setName(e.target.value)} />
            </Field>
            <Field label="Type">
              <p className="flex h-8 items-center text-sm">{detail.type}</p>
              <p className="text-xs text-muted-foreground">
                Fixed — a different type is a different connector.
              </p>
            </Field>
            <Field label="Direction" htmlFor="connector-direction">
              <select
                id="connector-direction"
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
            <DialFormFor adapter={adapter} typeName={detail.type} dial={dial} onChange={setDial} />
            <CredentialFields
              noCredential={noCredential}
              optional
              credentialManaged={detail.credentialManaged}
              slots={slots}
              primaryValue={primaryValue}
              onPrimaryValueChange={setPrimaryValue}
              secondaryValue={secondaryValue}
              onSecondaryValueChange={setSecondaryValue}
              problems={problems}
            />
          </div>
        ) : null}
        {step === 2 ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Environment" htmlFor="connector-environment">
              <Input id="connector-environment" value={environment} onChange={(e) => setEnvironment(e.target.value)} />
            </Field>
            <TenantField
              value={tenantId}
              onChange={setTenantId}
              hint="Moving a connector to another tenant needs the identity:write permission."
            />
            <Field label="Residency" htmlFor="connector-residency" className="sm:col-span-2">
              <Input id="connector-residency" value={residency} onChange={(e) => setResidency(e.target.value)} />
            </Field>
          </div>
        ) : null}
        {step === 3 ? (
          anyChange ? (
            <FormReviewSummary sections={[{ title: "Changes", items: reviewItems }]} />
          ) : (
            <p className="text-sm text-muted-foreground">Nothing has changed yet.</p>
          )
        ) : null}
      </FormStepLayout>
      <DeleteSection detail={detail} />
    </div>
  )
}

/** Removing the connector, kept apart from the form it would otherwise be mistaken for part of. */
function DeleteSection({ detail }: { detail: ConnectorDetail }) {
  const router = useRouter()
  const [open, setOpen] = React.useState(false)
  const inUse = detail.dependentPipelines.length
  return (
    <>
      <SectionCard
        size="sm"
        contentClassName="hidden"
        title="Delete connector"
        description={
          inUse > 0
            ? `Used by ${inUse} pipeline${inUse === 1 ? "" : "s"}; those must be deleted or moved first.`
            : "Removes the connector and its stored credential. This cannot be undone."
        }
        action={
          <Button
            size="sm"
            variant="outline"
            className="text-destructive hover:text-destructive"
            onClick={() => setOpen(true)}
          >
            Delete…
          </Button>
        }
      >
        {null}
      </SectionCard>
      <ConnectorDeleteDialog
        connector={detail}
        dependents={detail.dependentPipelines}
        open={open}
        onOpenChange={setOpen}
        onDeleted={() => router.push("/connectors")}
      />
    </>
  )
}

function EditHeader() {
  return (
    <PageHeader
      title="Edit Connector"
      description="Change a connector's settings; the connection is tested again after saving."
      actions={
        <Button variant="outline" size="sm" render={<Link href="/connectors" />}>
          Cancel
        </Button>
      }
    />
  )
}

function withStoredKinds(
  slots: CredentialSlots,
  primary: CredentialKind | null | undefined,
  secondary: CredentialKind | null | undefined
): CredentialSlots {
  if (!primary || Boolean(secondary) !== Boolean(slots.secondary)) return slots
  if (secondary && secondary === primary) return slots
  return {
    primary: { ...slots.primary, kind: primary },
    secondary: slots.secondary && secondary ? { ...slots.secondary, kind: secondary } : null,
  }
}
