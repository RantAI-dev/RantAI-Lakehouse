"use client"

import * as React from "react"
import Link from "next/link"
import { KeyRoundIcon, NetworkIcon, Trash2Icon, TriangleAlertIcon } from "lucide-react"
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
import { targetChanged } from "@/lib/connectors/target-identity"
import { connectorService } from "@/services"
import { isServiceError } from "@/services/errors"
import type {
  Connector,
  ConnectorDetail,
  ConnectorTestResult,
  ConnectorType,
  CredentialKind,
  IngestSpec,
  IngestSpecInput,
  SetConnectorCredentialRequest,
  UpdateConnectorInput,
} from "@/services/contracts/connectors"
import {
  CredentialFields,
  type CredentialSlots,
  DialFormFor,
  EditStepButton,
  Field,
  StepSectionHeading,
  credentialProblems,
  credentialSlotsFor,
  credentialSlotsKey,
  hostFromDial,
  normalizedCredential,
  usesNoCredential as needsNoCredential,
} from "./connector-form-parts"
import { ConnectorDeleteDialog } from "./connector-delete-dialog"
import { connectionReviewItems } from "./connector-review"
import { ScopeFields } from "./connector-scope-fields"
import { DirectionPicker, SelectedTypeSummary } from "./connector-type-picker"
import { DIRECTION_LABEL } from "./connectors-columns"

const STEPS: FormStep[] = [
  { id: "type", label: "Connector", description: "Name and direction" },
  { id: "connection", label: "Connection", description: "Where and how it signs in" },
  { id: "scope", label: "Tenant and residency" },
  { id: "review", label: "Review", description: "Confirm changes" },
]

type Loaded = { detail: ConnectorDetail; spec: IngestSpec; type: ConnectorType | null }

/** One part of a save, reported back to the user in order. A `note` on a
 * saved part says something the user should still know (e.g. a credential
 * that was stored without being tested). */
type SaveStep = { label: string; status: "done" | "failed"; message?: string; note?: string }

/**
 * Edit an existing connector with the same steps as creating one. A
 * connector's parts are owned by different routes, so a save is a short
 * sequence — basic fields (`PATCH`), credential, connection settings
 * (ingest spec), tenant — followed by a connection test. It stops at the
 * first failure and reports exactly which parts were saved, rather than a
 * bare "failed".
 *
 * A change of where the connector points (`SEC-14`: host, port, database,
 * endpoint, bucket, base URL, brokers…) is the exception to that sequence.
 * The stored credential is never sent to a new place, so the credential
 * inputs become required and travel WITH the connection settings in one
 * ingest-spec request, which the server tests and saves as one step.
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
  // A different sign-in method (REST bearer to basic, SFTP password to key)
  // reads a different credential, so the stored one cannot be kept.
  const signInChanged =
    credentialSlotsKey(credentialSlotsFor(adapter, originalDial)) !== credentialSlotsKey(credentialSlotsFor(adapter, dial))
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
  const dialChanged = JSON.stringify(dial ?? {}) !== JSON.stringify(originalDial)
  // SEC-14: pointing the connector somewhere else needs the credential again.
  // The server decides (its function is the authority); this only decides
  // what the form asks for.
  const repoints =
    dialChanged && targetChanged({ storedAdapter: spec.adapter ?? null, storedDial: originalDial, adapter, dial })
  const problems = credentialProblems({
    noCredential,
    optional: !signInChanged && !repoints,
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
  // The `host` column is only a label for a connector that dials from its
  // dial; a connector with no adapter yet, or a `files` one, still dials from
  // it, and the API refuses to change it here (SEC-14).
  const hostIsDialed = spec.adapter === null || spec.adapter === "files"
  if (dialChanged && !hostIsDialed) {
    const host = hostFromDial(adapter, dial)
    if (host) basic.host = host
  }
  const basicChanged = Object.keys(basic).length > 0
  const tenantChanged = tenantId !== "" && tenantId !== (detail.tenantId ?? "")
  const credentialTyped = !noCredential && (primaryValue !== "" || secondaryValue !== "")
  const anyChange = basicChanged || dialChanged || tenantChanged || credentialTyped

  const tenantNameOf = (id: string | null) => user?.tenants.find((t) => t.id === id)?.name ?? (id ? "Other tenant" : "Unassigned")
  const typeShown: ConnectorType = type ?? { name: detail.type, adapter, supported: true, docsUrl: null, unsupportedReason: null }

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
    const record = (label: string, run: () => Promise<string | void>) => async () => {
      try {
        const note = await run()
        steps.push({ label, status: "done", ...(note ? { note } : {}) })
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
    const values: Pick<SetConnectorCredentialRequest, "primary" | "secondary"> = {}
    if (credentialTyped) {
      values.primary = { kind: slots.primary.kind, value: normalizedCredential(slots.primary, primaryValue) }
      if (slots.secondary) {
        values.secondary = {
          kind: slots.secondary.kind,
          value: normalizedCredential(slots.secondary, secondaryValue),
        }
      }
    }
    const credential: SetConnectorCredentialRequest = {
      ...(dialChanged ? { dial: (dial ?? {}) as IngestSpec["dial"] } : {}),
      ...values,
    }
    const specInput: IngestSpecInput = {
      adapter: adapter as NonNullable<IngestSpec["adapter"]>,
      ingestMode: spec.ingestMode ?? (adapter === "cdc" ? "cdc" : "batch"),
      dial: (dial ?? {}) as IngestSpec["dial"],
      // Kept as they are: this page does not edit what is ingested.
      sourceObjects: spec.sourceObjects,
      ...(spec.scheduleCron ? { scheduleCron: spec.scheduleCron } : {}),
      // Pointing somewhere else: the credential goes with the settings, in
      // one request, and is never kept (SEC-14).
      ...(repoints && credentialTyped ? { credential: values } : {}),
    }
    // Never keep a credential in memory longer than the request needs it.
    setPrimaryValue("")
    setSecondaryValue("")

    const saveConnection = record(
      repoints ? "Connection settings and credential (tested against the new place first)" : "Connection settings",
      async () => {
        await connectorService.setIngestSpec(detail.id, specInput)
      }
    )
    const saveBasic = record("Name, direction, environment and residency", async () => {
      await connectorService.updateConnector(detail.id, basic)
    })
    const saveTenant = record("Tenant", async () => {
      await connectorService.assignTenant(detail.id, tenantId)
    })

    const sequence: (() => Promise<boolean>)[] = []
    if (repoints) {
      // Connection settings and credential first: if the server refuses them
      // (409, or the credential does not work at the new place), nothing else
      // has been saved.
      sequence.push(saveConnection)
      if (basicChanged) sequence.push(saveBasic)
    } else {
      // Order matters: the credential goes BEFORE the connection settings.
      // It is tested with the settings this edit makes (sent along with it),
      // and a sign-in method that needs a second credential (REST basic auth)
      // is only accepted once that credential exists.
      if (basicChanged) sequence.push(saveBasic)
      if (credentialTyped) {
        sequence.push(record("Credential (tested against the source first)", async () => {
          const saved = await connectorService.setCredential(detail.id, credential)
          return saved.verified
            ? undefined
            : `stored without a test: this connector type cannot be tested yet (${saved.message})`
        }))
      }
      if (dialChanged) sequence.push(saveConnection)
    }
    if (tenantChanged) sequence.push(saveTenant)

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
        <EditHeader connectorId={detail.id} />
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
        <EditHeader connectorId={detail.id} />
        <SectionCard
          title={failed ? "Some changes were not saved" : "Changes saved"}
          description={failed ? "Saving stopped at the first failure. Parts marked saved are in effect." : undefined}
        >
          <ul className="space-y-1.5 text-sm">
            {saveSteps.map((s) => (
              <li key={s.label} className={s.status === "done" ? "text-emerald-600 dark:text-emerald-400" : "text-destructive"}>
                {s.status === "done" ? "Saved" : "Not saved"} · {s.label}
                {s.message ? ` — ${s.message}` : ""}
                {s.note ? (
                  <span className="mt-0.5 flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-400">
                    <TriangleAlertIcon className="mt-px size-3.5 shrink-0" />
                    {s.note}
                  </span>
                ) : null}
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
            <Button size="sm" render={<Link href={`/connectors/${detail.id}`} />}>
              Back to connector
            </Button>
            <Button size="sm" variant="outline" onClick={onReload}>
              Edit again
            </Button>
          </div>
        </SectionCard>
      </div>
    )
  }

  // ── Review: what changes, before → after, grouped by step ──────────────
  const change = (label: string, before: string, after: string) => ({ label, value: `${before || "—"} → ${after || "—"}` })
  const connectorChanges = [
    ...(basic.name ? [change("Name", detail.name, basic.name)] : []),
    ...(basic.direction ? [change("Direction", DIRECTION_LABEL[detail.direction], DIRECTION_LABEL[basic.direction])] : []),
  ]
  const before = connectionReviewItems(adapter, originalDial)
  const after = connectionReviewItems(adapter, dial)
  const text = (value: React.ReactNode) => (typeof value === "string" ? value : "")
  const connectionChanges = [
    ...after.flatMap((item, i) =>
      text(item.value) !== text(before[i]?.value) ? [change(item.label, text(before[i]?.value), text(item.value))] : []
    ),
    ...(credentialTyped
      ? [
          {
            label: "Credential",
            value: `New ${slots.primary.label.toLowerCase()}${
              slots.secondary ? ` + ${slots.secondary.label.toLowerCase()}` : ""
            } · ${
              repoints ? "sent with the new connection settings and tested against them" : "tested before it replaces the current one"
            }`,
          },
        ]
      : []),
  ]
  const scopeChanges = [
    ...(tenantChanged ? [change("Tenant", tenantNameOf(detail.tenantId), tenantNameOf(tenantId))] : []),
    ...(basic.environment ? [change("Environment", detail.environment, basic.environment)] : []),
    ...(basic.residency !== undefined ? [change("Residency", detail.residency, basic.residency)] : []),
  ]
  const reviewSections = [
    { title: "Connector", step: 0, items: connectorChanges },
    { title: "Connection", step: 1, items: connectionChanges },
    { title: "Tenant and residency", step: 2, items: scopeChanges },
  ]
    .filter((section) => section.items.length > 0)
    .map((section) => ({
      title: section.title,
      items: section.items,
      action: <EditStepButton label={section.title.toLowerCase()} onClick={() => setStep(section.step)} />,
    }))

  return (
    <div className="flex flex-col gap-4">
      <EditHeader connectorId={detail.id} />
      <FormStepLayout
        steps={STEPS}
        currentIndex={step}
        onStepChange={setStep}
        canProceed={canProceed}
        onSubmit={handleSave}
        submitLabel="Save changes"
        submitting={saving}
        below={<DeleteSection detail={detail} />}
      >
        {step === 0 ? (
          <div className="space-y-5">
            <SelectedTypeSummary type={typeShown} />
            <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)]">
              <Field label="Name" htmlFor="connector-name">
                <Input
                  id="connector-name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  autoComplete="off"
                />
                <p className="text-xs text-muted-foreground">How this connector appears in lists and runs.</p>
              </Field>
              <Field label="Direction">
                <DirectionPicker value={direction} onChange={setDirection} />
              </Field>
            </div>
          </div>
        ) : null}
        {step === 1 ? (
          <div className="space-y-6">
            <section className="space-y-3">
              <StepSectionHeading
                icon={<NetworkIcon className="size-4" />}
                title="Connection details"
                description="Where the source lives. It must be reachable from the lakehouse server, not only from your browser."
              />
              <DialFormFor adapter={adapter} typeName={detail.type} dial={dial} onChange={setDial} />
            </section>
            <section className="space-y-3 border-t border-border pt-5">
              <StepSectionHeading
                icon={<KeyRoundIcon className="size-4" />}
                title="Credential"
                description={
                  repoints
                    ? "This points the connector at a different place, so the stored credential is not sent there. Enter it again."
                    : signInChanged
                      ? "The sign-in method changed, so the stored credential no longer fits. Enter the new one."
                      : "Leave blank to keep the current credential."
                }
              />
              <CredentialFields
                noCredential={noCredential}
                optional={!signInChanged && !repoints}
                credentialManaged={detail.credentialManaged}
                slots={slots}
                primaryValue={primaryValue}
                onPrimaryValueChange={setPrimaryValue}
                secondaryValue={secondaryValue}
                onSecondaryValueChange={setSecondaryValue}
                problems={problems}
              />
            </section>
          </div>
        ) : null}
        {step === 2 ? (
          <ScopeFields
            tenantDescription="Who can see and use this connector. Moving it to another tenant needs the identity:write permission."
            tenantId={tenantId}
            onTenantChange={setTenantId}
            environment={environment}
            onEnvironmentChange={setEnvironment}
            residency={residency}
            onResidencyChange={setResidency}
          />
        ) : null}
        {step === 3 ? (
          anyChange ? (
            <FormReviewSummary sections={reviewSections} />
          ) : (
            <p className="text-sm text-muted-foreground">
              Nothing has changed yet, so there is nothing to save. Change something in an earlier step and it shows
              up here as before → after.
            </p>
          )
        ) : null}
      </FormStepLayout>
    </div>
  )
}

/**
 * Removing the connector, kept apart from the form it would otherwise be
 * mistaken for part of. While pipelines still read from the connector the
 * API refuses the delete whatever it is asked, so the button is off and the
 * pipelines are named right here instead of behind it.
 */
function DeleteSection({ detail }: { detail: ConnectorDetail }) {
  const router = useRouter()
  const [open, setOpen] = React.useState(false)
  const dependents = detail.dependentPipelines
  return (
    <>
      <section aria-labelledby="delete-connector-title" className="rounded-xl border border-destructive/30">
        <div className="flex flex-col gap-3 px-5 py-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 items-start gap-3">
            <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-destructive/10 text-destructive">
              <Trash2Icon className="size-4" />
            </span>
            <div className="min-w-0">
              <h2 id="delete-connector-title" className="text-sm font-medium">
                Delete connector
              </h2>
              <p className="text-xs text-muted-foreground">
                Removes the connector and the credential stored for it. This cannot be undone.
              </p>
            </div>
          </div>
          <Button
            size="sm"
            variant="destructive"
            className="self-start sm:self-auto"
            disabled={dependents.length > 0}
            onClick={() => setOpen(true)}
          >
            Delete connector…
          </Button>
        </div>
        {dependents.length > 0 ? (
          <div className="flex items-start gap-2 rounded-b-xl border-t border-destructive/20 bg-destructive/5 px-5 py-2.5">
            <TriangleAlertIcon className="mt-px size-3.5 shrink-0 text-destructive" />
            <p className="text-xs">
              Still used by {dependents.length === 1 ? "the pipeline" : `${dependents.length} pipelines:`}{" "}
              {dependents.map((d, i) => (
                <React.Fragment key={d.id}>
                  {i > 0 ? ", " : null}
                  <Link href={`/pipelines/${d.id}`} className="font-medium text-primary hover:underline">
                    {d.name}
                  </Link>
                </React.Fragment>
              ))}
              . Delete {dependents.length === 1 ? "it" : "them"} or point {dependents.length === 1 ? "it" : "them"} at
              another connector first.
            </p>
          </div>
        ) : null}
      </section>
      <ConnectorDeleteDialog
        connector={detail}
        dependents={dependents}
        open={open}
        onOpenChange={setOpen}
        onDeleted={() => router.push("/connectors")}
      />
    </>
  )
}

/** Cancel goes back to the connector's page, where "Edit" was pressed. */
function EditHeader({ connectorId }: { connectorId: string }) {
  return (
    <PageHeader
      title="Edit Connector"
      description="Change a connector's settings; the connection is tested again after saving."
      actions={
        <Button variant="outline" size="sm" render={<Link href={`/connectors/${connectorId}`} />}>
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
