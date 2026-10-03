"use client"

import * as React from "react"
import Link from "next/link"
import { KeyRoundIcon, NetworkIcon } from "lucide-react"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { FormReviewSummary } from "@/components/patterns/form-review-summary"
import { FormStepLayout, type FormStep } from "@/components/patterns/form-step-layout"
import { PageHeader } from "@/components/patterns/page-header"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { connectorService } from "@/services"
import type { Connector, CredentialSpec } from "@/services/contracts/connectors"
import {
  CredentialFields,
  DialFormFor,
  Field,
  EditStepButton,
  StepSectionHeading,
  credentialProblems,
  credentialSlotsFor,
  credentialSlotsKey,
  hostFromDial,
  normalizedCredential,
  usesNoCredential as needsNoCredential,
} from "./connector-form-parts"
import { DIRECTION_LABEL } from "./connectors-columns"
import { connectionReviewItems } from "./connector-review"
import { ScopeFields } from "./connector-scope-fields"
import { ConnectorIngestPanel } from "./connector-ingest-panel"
import { ConnectorTypePicker, DirectionPicker, SelectedTypeSummary } from "./connector-type-picker"

// Only 4 pre-creation steps, not the 5 this file used to have: the old
// "discover" step was a capability-toggle placeholder with no real
// backend behind it. Real discovery (POST /api/connectors/{id}/discover)
// and ingest-spec configuration (PUT .../ingest-spec) both need a
// connector id that does not exist until creation succeeds -- the exact
// reason the real connection test already only ever ran post-creation in
// this file. Discovery/ingest-spec/run now live in that same
// post-creation panel instead of pretending to work before an id exists.
const STEPS: FormStep[] = [
  { id: "type", label: "Type", description: "Connector kind" },
  { id: "connection", label: "Connection", description: "Dial and secret" },
  { id: "scope", label: "Tenant and residency" },
  { id: "review", label: "Review", description: "Confirm" },
]

export function ConnectorCreatePage() {
  const [step, setStep] = React.useState(0)
  const types = useService(connectorService.listTypes)
  const [name, setName] = React.useState("")
  const [selectedTypeName, setSelectedTypeName] = React.useState<string | null>(null)
  const [direction, setDirection] = React.useState<Connector["direction"]>("source")
  const [dial, setDial] = React.useState<Record<string, unknown> | null>(null)
  // Write-only (ADR 0002 Addendum 4): sent once in the create request,
  // cleared right after, never rendered back.
  const [primaryValue, setPrimaryValue] = React.useState("")
  const [secondaryValue, setSecondaryValue] = React.useState("")
  const [environment, setEnvironment] = React.useState("production")
  const { user, activeTenantId } = useAuth()
  // Defaults to the tenant the user is acting as — the one the connector
  // list shows — so a new connector is visible to its creator.
  const [tenantId, setTenantId] = React.useState(activeTenantId ?? "")
  const tenantName = user?.tenants.find((t) => t.id === tenantId)?.name ?? ""
  const [residency, setResidency] = React.useState("")
  const [createdId, setCreatedId] = React.useState<string | null>(null)
  // `false` when the connection settings could not be saved, so the test
  // was not run (it would have dialed nothing meaningful).
  const [testSkipped, setTestSkipped] = React.useState(false)

  const selectedType = types.data?.find((t) => t.name === selectedTypeName) ?? null
  const adapter = selectedType?.adapter ?? null
  // An unauthenticated Kafka connector still carries a derived primary
  // name (the create body requires one), but nothing resolves it, so the
  // form asks for no credential at all.
  const usesNoCredential = needsNoCredential(adapter, dial)

  // The auth state may load after the first render.
  React.useEffect(() => {
    if (tenantId === "" && activeTenantId) setTenantId(activeTenantId)
  }, [activeTenantId, tenantId])

  // Default the selection to the first SUPPORTED type once the list
  // loads, so the wizard never opens sitting on a disabled option a user
  // could not have picked themselves.
  React.useEffect(() => {
    if (selectedTypeName === null && types.status === "success") {
      const firstSupported = types.data.find((t) => t.supported)
      if (firstSupported) setSelectedTypeName(firstSupported.name)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [types.status])

  // A new type means a new dial shape -- never carry the previous type's
  // fields into a struct that will reject them as unknown. Keyed on
  // `selectedTypeName` too, not just `adapter`: PostgreSQL, MySQL and
  // Oracle share the `sql` adapter (`DialFormFor`'s `typeName` doc
  // comment) but each starts from its own driver -- carrying a postgres
  // dial's `driver: "postgres"` into the MySQL or Oracle form would be
  // exactly the stale-shape bug this effect exists to prevent.
  React.useEffect(() => {
    setDial(null)
  }, [adapter, selectedTypeName])

  // Which credential fields the source reads follows from the adapter and
  // the auth type chosen in the connection form (REST basic needs a
  // username and a password, SFTP public-key auth a private key, ...).
  // A value typed for one layout never carries into another.
  const slots = credentialSlotsFor(adapter, dial)
  const slotsKey = credentialSlotsKey(slots)
  React.useEffect(() => {
    setPrimaryValue("")
    setSecondaryValue("")
  }, [slotsKey])

  const create = useServiceAction(
    withNotify(
      { success: "Connector created", error: "Failed to create connector" },
      (signal, input: Parameters<typeof connectorService.createConnector>[0]) =>
        connectorService.createConnector(input, signal)
    )
  )
  // The real probe (POST /api/connectors/{id}/test) needs a connector id, so
  // it can only run after `create` succeeds.
  const test = useServiceAction((signal, id: string) => connectorService.testConnection(id, signal))
  // Saves only the connection settings; which tables to ingest is chosen
  // after creation, in the same panel the connector's detail drawer uses.
  const saveSpec = useServiceAction((signal, id: string) =>
    connectorService.setIngestSpec(
      id,
      {
        adapter: adapter ?? "",
        ingestMode: adapter === "cdc" ? "cdc" : "batch",
        dial: dial ?? {},
        sourceObjects: [],
      },
      signal
    )
  )

  const problems = credentialProblems({
    noCredential: usesNoCredential,
    optional: false,
    slots,
    primaryValue,
    secondaryValue,
  })

  const canProceed =
    (step === 0 && Boolean(name.trim() && selectedType?.supported)) ||
    (step === 1 && problems.primary === null && problems.secondary === null) ||
    (step === 2 && Boolean(environment.trim() && tenantId && residency.trim())) ||
    step === 3

  async function handleSubmit() {
    if (!selectedType || !adapter) return
    // Always `managed` (ADR 0002 Addendum 4): the user supplies the value
    // here and the server stores it. Operator-provisioned env/file
    // credentials remain an API-only option for deployments that want
    // them; this form does not offer them.
    const credential: CredentialSpec = {
      source: "managed",
      primary: slots.primary.kind,
      ...(slots.secondary ? { secondary: slots.secondary.kind } : {}),
      ...(usesNoCredential
        ? {}
        : {
            values: {
              primary: normalizedCredential(slots.primary, primaryValue),
              ...(slots.secondary
                ? { secondary: normalizedCredential(slots.secondary, secondaryValue) }
                : {}),
            },
          }),
    }
    const result = await create.run({
      name: name.trim(),
      type: selectedType.name,
      direction,
      host: hostFromDial(adapter, dial),
      credential,
      environment: environment.trim(),
      tenant: tenantName,
      tenantId,
      residency: residency.trim(),
      capabilities: [],
    })
    // Never keep a credential in memory longer than the request needs it.
    setPrimaryValue("")
    setSecondaryValue("")
    if (result) {
      setCreatedId(result.id)
      // The connection settings (`dial`) are only stored by the ingest
      // spec, so save it BEFORE testing: a test run first would dial the
      // connector without its host/port/user/database and fail for a
      // reason that has nothing to do with the credential.
      const saved = await saveSpec.run(result.id)
      if (saved) {
        await test.run(result.id)
      } else {
        setTestSkipped(true)
      }
    }
  }

  // Creation succeeded: show the real test outcome, then let the user pick
  // the tables to ingest and run them, instead of redirecting past any of
  // it.
  if (createdId) {
    return (
      <div className="flex flex-col gap-4">
        <PageHeader
          title="New Connector"
          description="Configure a source or sink, verify connectivity, and register what it ingests."
          actions={
            <Button variant="outline" size="sm" render={<Link href="/connectors" />}>
              View connectors
            </Button>
          }
        />
        <SectionCard
          title="Connector created"
          description={`"${name.trim()}" was created. Here is the result of the connection test.`}
        >
          <div className="space-y-3">
            {testSkipped ? (
              <div className="space-y-1.5">
                <p className="text-sm text-destructive">
                  The connection test was not run: the connection settings could not be saved.
                  {saveSpec.status === "error" ? ` ${saveSpec.error.message}` : ""}
                </p>
                <p className="text-xs text-muted-foreground">
                  Correct them from{" "}
                  <Link href={`/connectors/${createdId}/edit`} className="text-primary hover:underline">
                    Edit
                  </Link>
                  , then run the test again.
                </p>
              </div>
            ) : test.status === "pending" ? (
              <p className="text-sm text-muted-foreground">Testing connection…</p>
            ) : test.status === "error" ? (
              <ErrorState error={test.error} onRetry={() => test.run(createdId)} />
            ) : test.data ? (
              <p
                className={
                  !test.data.supported
                    ? "text-sm text-muted-foreground"
                    : test.data.ok
                      ? "text-sm text-emerald-600 dark:text-emerald-400"
                      : "text-sm text-destructive"
                }
              >
                {!test.data.supported
                  ? `This connector type cannot be tested yet · ${test.data.message}`
                  : test.data.ok
                    ? `Connection test passed · ${test.data.message}`
                    : `Connection test failed · ${test.data.message}`}
              </p>
            ) : null}
            <div className="flex flex-wrap gap-2">
              <Button size="sm" render={<Link href="/connectors" />}>
                View connectors
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={test.status === "pending"}
                onClick={() => {
                  setTestSkipped(false)
                  test.run(createdId)
                }}
              >
                Run test again
              </Button>
            </div>
          </div>
        </SectionCard>

        {!testSkipped ? (
          <SectionCard title="What to ingest" description="Pick the tables this connector copies into Bronze, then run it.">
            <ConnectorIngestPanel connectorId={createdId} connectorName={name.trim()} />
          </SectionCard>
        ) : null}
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="New Connector"
        description="Configure a source or sink with discovery and residency scope, then verify connectivity."
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
          <div className="space-y-6">
            {/* A file is not a registered source: it has its own page
                (DATA-9, `docs/core/features/upload-file.md`). */}
            <p className="text-sm text-muted-foreground">
              <Link href="/connectors/upload" className="text-primary underline-offset-4 hover:underline">
                Have a file instead? Upload a CSV or TSV
              </Link>
            </p>
            {types.status === "loading" ? (
              <LoadingSkeleton rows={4} />
            ) : types.status === "error" ? (
              <ErrorState error={types.error} onRetry={types.reload} />
            ) : (
              <ConnectorTypePicker types={types.data} value={selectedTypeName} onChange={setSelectedTypeName} />
            )}
            <div className="grid gap-4 border-t border-border pt-5 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)]">
              <Field label="Name" htmlFor="connector-name">
                <Input
                  id="connector-name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder={selectedType ? `e.g. ${selectedType.name} orders` : "e.g. core orders"}
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
            {selectedType ? <SelectedTypeSummary type={selectedType} onChange={() => setStep(0)} /> : null}
            <section className="space-y-3">
              <StepSectionHeading
                icon={<NetworkIcon className="size-4" />}
                title="Connection details"
                description="Where the source lives. It must be reachable from the lakehouse server, not only from your browser."
              />
              <DialFormFor adapter={adapter} typeName={selectedType?.name ?? null} dial={dial} onChange={setDial} />
            </section>
            <section className="space-y-3 border-t border-border pt-5">
              <StepSectionHeading
                icon={<KeyRoundIcon className="size-4" />}
                title="Credential"
                description="The secret lakehouse signs in with."
              />
              <CredentialFields
                noCredential={usesNoCredential}
                optional={false}
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
            tenantDescription="Who can see and use this connector. Only members of the chosen tenant see it in their connector list."
            tenantId={tenantId}
            onTenantChange={setTenantId}
            environment={environment}
            onEnvironmentChange={setEnvironment}
            residency={residency}
            onResidencyChange={setResidency}
          />
        ) : null}
        {step === 3 ? (
          <div className="space-y-6">
            <FormReviewSummary
              sections={[
                {
                  title: "Connector",
                  action: <EditStepButton label="connector" onClick={() => setStep(0)} />,
                  items: [
                    { label: "Name", value: name.trim() },
                    { label: "Type", value: selectedType?.name ?? "" },
                    { label: "Direction", value: DIRECTION_LABEL[direction] },
                  ],
                },
                {
                  title: "Connection",
                  action: <EditStepButton label="connection" onClick={() => setStep(1)} />,
                  items: [
                    ...connectionReviewItems(adapter, dial),
                    {
                      label: "Credential",
                      value: usesNoCredential
                        ? "None"
                        : `${slots.primary.label}${
                            slots.secondary ? ` + ${slots.secondary.label.toLowerCase()}` : ""
                          } · entered, stored by lakehouse`,
                    },
                  ],
                },
                {
                  title: "Tenant and residency",
                  action: <EditStepButton label="tenant and residency" onClick={() => setStep(2)} />,
                  items: [
                    { label: "Tenant", value: tenantName },
                    { label: "Environment", value: environment.trim() },
                    { label: "Residency", value: residency.trim() },
                  ],
                },
              ]}
            />

            <section className="space-y-3 rounded-lg border border-border p-4">
              <h3 className="text-sm font-medium">What happens when you create it</h3>
              <ol className="list-decimal space-y-1.5 pl-5 text-sm text-muted-foreground">
                <li>
                  The connector is added to <span className="font-medium text-foreground">{tenantName || "the tenant"}</span>{" "}
                  and appears in its connector list.
                </li>
                <li>
                  {usesNoCredential
                    ? "No credential is stored: this connection authenticates with none."
                    : "The credential is stored by lakehouse and never shown again."}
                </li>
                <li>The connection settings are saved, then the connection is tested. You see the result here.</li>
                <li>
                  You then pick the tables to copy into Bronze and run them. Nothing is ingested until you do.
                </li>
              </ol>
            </section>
          </div>
        ) : null}
        {create.status === "error" ? (
          <p className="text-sm text-destructive">{create.error.message}</p>
        ) : null}
      </FormStepLayout>
    </div>
  )
}
