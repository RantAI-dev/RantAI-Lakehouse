"use client"

import * as React from "react"
import Link from "next/link"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { FormReviewSummary } from "@/components/patterns/form-review-summary"
import { FormStepLayout, type FormStep } from "@/components/patterns/form-step-layout"
import { PageHeader } from "@/components/patterns/page-header"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { CdcDialForm } from "@/features/connectors/dial-forms/cdc-dial-form"
import { FilesDialForm } from "@/features/connectors/dial-forms/files-dial-form"
import { KafkaDialForm } from "@/features/connectors/dial-forms/kafka-dial-form"
import { MongoDialForm } from "@/features/connectors/dial-forms/mongo-dial-form"
import { OracleDialForm } from "@/features/connectors/dial-forms/oracle-dial-form"
import { RestDialForm } from "@/features/connectors/dial-forms/rest-dial-form"
import { SftpDialForm } from "@/features/connectors/dial-forms/sftp-dial-form"
import { SheetsDialForm } from "@/features/connectors/dial-forms/sheets-dial-form"
import { SqlDialForm } from "@/features/connectors/dial-forms/sql-dial-form"
import { useService, useServiceAction } from "@/hooks/use-service"
import { cn } from "@/lib/utils"
import { connectorService } from "@/services"
import type {
  CdcDial,
  Connector,
  CredentialKind,
  CredentialSource,
  CredentialSpec,
  FilesDial,
  KafkaDial,
  MongoDial,
  RestDial,
  SftpDial,
  SheetsDial,
  SourceObject,
  SqlDial,
} from "@/services/contracts/connectors"

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

/**
 * `CreateConnectorInput.host` still exists on the base connector row
 * (used by `connector_probe`'s own connectivity test, a DIFFERENT probe
 * than ingest discovery) -- derive it from whichever dial field is this
 * adapter's own "where do I connect" field, rather than asking for the
 * same value twice.
 */
function hostFromDial(adapter: string | null, dial: Record<string, unknown> | null): string {
  if (!dial) return ""
  switch (adapter) {
    case "sql":
    case "cdc":
    case "sftp":
      return typeof dial.host === "string" ? dial.host : ""
    case "files":
      return typeof dial.bucket === "string" ? dial.bucket : ""
    case "rest":
      return typeof dial.baseUrl === "string" ? dial.baseUrl : ""
    case "sheets":
      return typeof dial.spreadsheetId === "string" ? dial.spreadsheetId : ""
    case "mongodb":
      return Array.isArray(dial.hosts) && typeof dial.hosts[0] === "string" ? (dial.hosts[0] as string) : ""
    case "kafka":
      return Array.isArray(dial.bootstrapServers) && typeof dial.bootstrapServers[0] === "string"
        ? (dial.bootstrapServers[0] as string)
        : ""
    default:
      return ""
  }
}

const CREDENTIAL_KIND_OPTIONS: { value: CredentialKind; label: string }[] = [
  { value: "password", label: "Password" },
  { value: "secret_key", label: "Secret key" },
  { value: "access_key", label: "Access key" },
  { value: "api_key", label: "API key" },
  { value: "token", label: "Token" },
  { value: "private_key", label: "Private key" },
]

/**
 * A sensible default primary/secondary kind per adapter -- the server
 * derives the actual reference NAME from the connector's own id (ADR 0002
 * Addendum 3), so this only picks which fixed suffix each slot uses.
 * `sql`/`cdc`/`mongodb`/`sftp`/`kafka` connectors dial with a single
 * password; a `files` (S3-shaped) connector needs an access-key/
 * secret-key pair; a `rest` connector most commonly authenticates with an
 * API key or bearer token. The user can still change either select --
 * this only seeds the initial value.
 */
// mongodb/oracle(sql)/kafka(sasl_plain)/sftp(password) all fall into the
// `default` "password" case below, same as the pre-Tier-2 sql/cdc
// adapters -- no adapter-specific branch needed for them. `sftp`'s
// `public_key` auth kind needs `private_key` instead; the user switches
// the primary select themselves after picking that auth type in the dial
// form (this seed only picks a starting value, per this function's own
// doc comment above). `kafka`'s `none` auth needs no credential at all
// (`secret_field_names` maps `("kafka", "none")` to no fields) but
// `CreateConnectorInput.credential.primary`/`CredentialSpec::primary` is
// a REQUIRED field server-side -- there is no honest "no credential" kind
// to send. Rather than invent one, this seeds `password` like every other
// adapter; the derived name is simply never provisioned for a `none`
// Kafka connector, and "Test"/ingest never reads it (the adapter's own
// `secret_field_names` lookup returns no fields, so nothing is resolved).
function defaultCredentialForAdapter(adapter: string | null): { primary: CredentialKind; secondary: CredentialKind | null } {
  switch (adapter) {
    case "files":
      return { primary: "access_key", secondary: "secret_key" }
    case "rest":
      return { primary: "api_key", secondary: null }
    default:
      return { primary: "password", secondary: null }
  }
}

function DialFormFor({
  adapter,
  typeName,
  dial,
  onChange,
}: {
  adapter: string | null
  /** `selectedType.name` — needed alongside `adapter` because Oracle
   * dials with the same `sql` adapter/`SqlDial` shape every other SQL
   * driver does (`SqlDriver::Oracle`, `ingest_spec.rs`), not a distinct
   * `adapter` value; `adapter` alone cannot tell Oracle apart from
   * PostgreSQL/MySQL/SQL Server. */
  typeName: string | null
  dial: Record<string, unknown> | null
  onChange: (next: Record<string, unknown>) => void
}) {
  // Oracle is the one type whose dial form is picked by NAME, not by
  // `adapter` alone -- see this function's `typeName` doc comment above.
  if (adapter === "sql" && typeName === "Oracle") {
    return (
      <OracleDialForm
        value={dial as unknown as SqlDial | null}
        onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
      />
    )
  }
  switch (adapter) {
    case "sql":
      return (
        <SqlDialForm
          value={dial as unknown as SqlDial | null}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
        />
      )
    case "cdc":
      return (
        <CdcDialForm
          value={dial as unknown as CdcDial | null}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
        />
      )
    case "files":
      return (
        <FilesDialForm
          value={dial as unknown as FilesDial | null}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
        />
      )
    case "rest":
      return (
        <RestDialForm
          value={dial as unknown as RestDial | null}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
        />
      )
    case "sheets":
      return (
        <SheetsDialForm
          value={dial as unknown as SheetsDial | null}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
        />
      )
    case "mongodb":
      return (
        <MongoDialForm
          value={dial as unknown as MongoDial | null}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
        />
      )
    case "kafka":
      return (
        <KafkaDialForm
          value={dial as unknown as KafkaDial | null}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
        />
      )
    case "sftp":
      return (
        <SftpDialForm
          value={dial as unknown as SftpDial | null}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
        />
      )
    default:
      return (
        <p className="text-sm text-muted-foreground">
          This type has no adapter registered yet -- pick a supported type to configure its connection.
        </p>
      )
  }
}

export function ConnectorCreatePage() {
  const [step, setStep] = React.useState(0)
  const types = useService(connectorService.listTypes)
  const [name, setName] = React.useState("")
  const [selectedTypeName, setSelectedTypeName] = React.useState<string | null>(null)
  const [direction, setDirection] = React.useState<Connector["direction"]>("source")
  const [dial, setDial] = React.useState<Record<string, unknown> | null>(null)
  const [credentialSource, setCredentialSource] = React.useState<CredentialSource>("env")
  const [credentialPrimary, setCredentialPrimary] = React.useState<CredentialKind>("password")
  const [credentialSecondary, setCredentialSecondary] = React.useState<CredentialKind | null>(null)
  const [environment, setEnvironment] = React.useState("production")
  const [tenant, setTenant] = React.useState("")
  const [residency, setResidency] = React.useState("")
  const [createdId, setCreatedId] = React.useState<string | null>(null)
  const [createdCredential, setCreatedCredential] = React.useState<{
    primary: string
    secondary: string | null
  } | null>(null)
  const [sourceObjects, setSourceObjects] = React.useState<SourceObject[]>([])
  const [scheduleCron, setScheduleCron] = React.useState("")
  const [runNow, setRunNow] = React.useState(false)

  const selectedType = types.data?.find((t) => t.name === selectedTypeName) ?? null
  const adapter = selectedType?.adapter ?? null

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

  // A new adapter means a new dial shape -- never carry the previous
  // adapter's fields into a struct that will reject them as unknown. Also
  // reseed the credential kind defaults for the new adapter (the user can
  // still override either select afterward). Keyed on `selectedTypeName`
  // too, not just `adapter`: switching PostgreSQL <-> Oracle keeps the
  // same `sql` adapter (`DialFormFor`'s `typeName` doc comment) but
  // renders a DIFFERENT form (`OracleDialForm`'s driver-locked shape vs.
  // `SqlDialForm`'s driver select) -- carrying a postgres dial's `driver:
  // "postgres"` into the Oracle form (or vice versa) would be exactly the
  // stale-shape bug this effect exists to prevent.
  React.useEffect(() => {
    setDial(null)
    const defaults = defaultCredentialForAdapter(adapter)
    setCredentialPrimary(defaults.primary)
    setCredentialSecondary(defaults.secondary)
  }, [adapter, selectedTypeName])

  const create = useServiceAction((signal, input: Parameters<typeof connectorService.createConnector>[0]) =>
    connectorService.createConnector(input, signal)
  )
  // The real probe (POST /api/connectors/{id}/test) needs a connector id, so
  // it can only run after `create` succeeds.
  const test = useServiceAction((signal, id: string) => connectorService.testConnection(id, signal))
  const saveSpec = useServiceAction((signal, id: string, objects: SourceObject[]) =>
    connectorService.setIngestSpec(
      id,
      {
        adapter: adapter ?? "",
        ingestMode: adapter === "cdc" ? "cdc" : "batch",
        dial: dial ?? {},
        sourceObjects: objects,
        scheduleCron: scheduleCron.trim() || undefined,
      },
      signal
    )
  )
  const discover = useServiceAction((signal, id: string) => connectorService.discoverConnector(id, signal))
  const run = useServiceAction((signal, id: string) => connectorService.runIngest(id, signal))

  const canProceed =
    (step === 0 && Boolean(name.trim() && selectedType?.supported)) ||
    (step === 1 && Boolean(credentialPrimary)) ||
    (step === 2 && Boolean(environment.trim() && tenant.trim() && residency.trim())) ||
    step === 3

  async function handleSubmit() {
    if (!selectedType || !adapter) return
    const credential: CredentialSpec = {
      source: credentialSource,
      primary: credentialPrimary,
      ...(credentialSecondary ? { secondary: credentialSecondary } : {}),
    }
    const result = await create.run({
      name: name.trim(),
      type: selectedType.name,
      direction,
      host: hostFromDial(adapter, dial),
      credential,
      environment: environment.trim(),
      tenant: tenant.trim(),
      residency: residency.trim(),
      capabilities: [],
    })
    if (result) {
      setCreatedId(result.id)
      setCreatedCredential(result.credential)
      await test.run(result.id)
      const saved = await saveSpec.run(result.id, [])
      if (saved && runNow) {
        await run.run(result.id)
      }
    }
  }

  function toggleObject(name: string) {
    const already = sourceObjects.some((o) => o.name === name)
    setSourceObjects(
      already ? sourceObjects.filter((o) => o.name !== name) : [...sourceObjects, { name, target: name }]
    )
  }

  // Creation succeeded: show the real test outcome, then let the user
  // discover objects and confirm the ingest spec, instead of redirecting
  // past any of it.
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
        {createdCredential ? (
          <SectionCard
            title="Provision these credentials"
            description="Shown once, now — the server derived these names from this connector's own id (ADR 0002 Addendum 3). They are not stored or shown again; write them down before leaving this page."
          >
            <div className="space-y-2 text-sm">
              <div>
                <span className="text-muted-foreground">Primary: </span>
                <code className="rounded bg-muted px-1.5 py-0.5">{createdCredential.primary}</code>
              </div>
              {createdCredential.secondary ? (
                <div>
                  <span className="text-muted-foreground">Secondary: </span>
                  <code className="rounded bg-muted px-1.5 py-0.5">{createdCredential.secondary}</code>
                </div>
              ) : null}
              <p className="text-xs text-muted-foreground">
                An <code>env:</code> name needs a restart of the processes that read it; a{" "}
                <code>file:</code> name can be replaced in place. Until provisioned, &quot;Test&quot; will
                report the credential as unresolvable, honestly.
              </p>
            </div>
          </SectionCard>
        ) : null}
        <SectionCard
          title="Connector created"
          description={`"${name.trim()}" was created. Here is the result of the connection test.`}
        >
          <div className="space-y-3">
            {test.status === "pending" ? (
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
                onClick={() => test.run(createdId)}
              >
                Run test again
              </Button>
            </div>
          </div>
        </SectionCard>

        {adapter === "sql" || adapter === "cdc" ? (
          <SectionCard
            title="Discover source objects"
            description="List this connector's tables so you can pick what to ingest."
          >
            <div className="space-y-3">
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={discover.status === "pending"}
                onClick={() => discover.run(createdId)}
              >
                {discover.status === "pending" ? "Discovering…" : "Discover"}
              </Button>
              {discover.status === "error" ? <ErrorState error={discover.error} /> : null}
              {discover.data && !discover.data.supported ? (
                <p className="text-sm text-muted-foreground">
                  Discovery is not supported for this connector · {discover.data.reason}
                </p>
              ) : null}
              {discover.data?.supported ? (
                <ul className="space-y-1.5">
                  {discover.data.objects.map((object) => {
                    const checked = sourceObjects.some((o) => o.name === object.name)
                    return (
                      <li key={object.name} className="flex items-center gap-2 text-sm">
                        <input
                          id={`discover-object-${object.name}`}
                          type="checkbox"
                          checked={checked}
                          onChange={() => toggleObject(object.name)}
                        />
                        <Label htmlFor={`discover-object-${object.name}`} className="font-normal">
                          {object.name}
                          <span className="ml-1 text-xs text-muted-foreground">
                            ({object.columns.length} columns)
                          </span>
                        </Label>
                      </li>
                    )
                  })}
                </ul>
              ) : null}
            </div>
          </SectionCard>
        ) : null}

        <SectionCard title="Ingest spec" description="Schedule and launch, or leave unscheduled until ready.">
          <div className="space-y-3">
            <div className="max-w-xs space-y-1.5">
              <Label htmlFor="schedule-cron">Schedule (cron, optional)</Label>
              <Input
                id="schedule-cron"
                value={scheduleCron}
                onChange={(e) => setScheduleCron(e.target.value)}
                placeholder="0 * * * *"
              />
            </div>
            <div className="flex flex-wrap gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={saveSpec.status === "pending"}
                onClick={() => saveSpec.run(createdId, sourceObjects)}
              >
                {saveSpec.status === "pending" ? "Saving…" : "Save ingest spec"}
              </Button>
              <Button
                type="button"
                size="sm"
                disabled={run.status === "pending"}
                onClick={() => run.run(createdId)}
              >
                {run.status === "pending" ? "Launching…" : "Run now"}
              </Button>
            </div>
            {saveSpec.status === "error" ? <ErrorState error={saveSpec.error} /> : null}
            {/* An honest supported:false (e.g. every cdc run -- ADR 0008)
                renders as inline text, never an error toast. */}
            {run.status === "error" ? <ErrorState error={run.error} /> : null}
            {run.data && run.data.supported === false ? (
              <p className="text-sm text-muted-foreground">{run.data.reason}</p>
            ) : null}
            {run.data?.runId ? (
              <p className="text-sm text-emerald-600 dark:text-emerald-400">Launched run {run.data.runId}</p>
            ) : null}
          </div>
        </SectionCard>
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
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Name" className="sm:col-span-2">
              <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="postgres core CDC" />
            </Field>
            <Field label="Type" className="sm:col-span-2">
              {types.status === "loading" ? (
                <LoadingSkeleton rows={2} />
              ) : types.status === "error" ? (
                <ErrorState error={types.error} onRetry={types.reload} />
              ) : (
                <select
                  className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                  value={selectedTypeName ?? ""}
                  onChange={(e) => setSelectedTypeName(e.target.value)}
                >
                  {types.data.map((t) => (
                    <option
                      key={t.name}
                      value={t.name}
                      disabled={!t.supported}
                      title={t.supported ? undefined : "not yet supported"}
                    >
                      {t.name}
                      {t.supported ? "" : " (not yet supported)"}
                    </option>
                  ))}
                </select>
              )}
              {/* rust/migrations/0035_connector_type.sql seeds Google
                  Sheets supported = true (the adapter and wizard exist),
                  but this build's sheets ingest adapter reports
                  unsupported for every run (WS3 item 23) -- shown here so
                  a user does not pick "available" on the strength of the
                  registry row alone and then discover, only after
                  creating a connector and clicking Run, that no run can
                  ever move data. */}
              {selectedType?.adapter === "sheets" ? (
                <p className="text-xs text-muted-foreground">
                  Google Sheets connections can be configured and tested, but this build&apos;s ingest adapter cannot
                  yet move data for this type — a triggered run will report unsupported, honestly, rather than
                  fail silently.
                </p>
              ) : null}
              {selectedType?.docsUrl ? (
                <a
                  href={selectedType.docsUrl}
                  target="_blank"
                  rel="noreferrer"
                  className="text-xs text-primary underline"
                >
                  Documentation
                </a>
              ) : null}
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
            <DialFormFor adapter={adapter} typeName={selectedType?.name ?? null} dial={dial} onChange={setDial} />
            <div className="grid gap-3 sm:grid-cols-2">
              <Field label="Credential source">
                <select
                  className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                  value={credentialSource}
                  onChange={(e) => setCredentialSource(e.target.value as CredentialSource)}
                >
                  <option value="env">Environment variable</option>
                  <option value="file">Mounted file</option>
                </select>
              </Field>
              <Field label="Primary credential kind">
                <select
                  className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                  value={credentialPrimary}
                  onChange={(e) => setCredentialPrimary(e.target.value as CredentialKind)}
                >
                  {CREDENTIAL_KIND_OPTIONS.map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
              </Field>
              {adapter === "files" ? (
                <Field label="Secondary credential kind" className="sm:col-span-2">
                  <select
                    className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                    value={credentialSecondary ?? ""}
                    onChange={(e) =>
                      setCredentialSecondary((e.target.value || null) as CredentialKind | null)
                    }
                  >
                    <option value="">None</option>
                    {CREDENTIAL_KIND_OPTIONS.map((o) => (
                      <option key={o.value} value={o.value}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                </Field>
              ) : null}
            </div>
            <p className="text-xs text-muted-foreground">
              The server assigns the actual credential reference name from this connector&apos;s own
              id once it is created (ADR 0002 Addendum 3) — you choose only where the value will
              live and which kind of credential it is; the name to provision is shown after
              creation.
            </p>
          </div>
        ) : null}
        {step === 2 ? (
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
        {step === 3 ? (
          <FormReviewSummary
            sections={[
              {
                title: "Connector",
                items: [
                  { label: "Name", value: name },
                  { label: "Type", value: selectedType?.name ?? "" },
                  { label: "Direction", value: direction },
                  {
                    label: "Credential",
                    value: `${credentialSource}: ${credentialPrimary}${credentialSecondary ? ` + ${credentialSecondary}` : ""}`,
                  },
                ],
              },
              {
                title: "Scope",
                items: [
                  { label: "Environment", value: environment },
                  { label: "Tenant", value: tenant },
                  { label: "Residency", value: residency },
                ],
              },
            ]}
          />
        ) : null}
        {step === 3 ? (
          <div className="flex items-center gap-2">
            <input
              id="run-now"
              type="checkbox"
              checked={runNow}
              onChange={(e) => setRunNow(e.target.checked)}
            />
            <Label htmlFor="run-now" className="font-normal">
              Run now, right after creation
              {adapter === "cdc" ? (
                <span className="ml-1 text-xs text-muted-foreground">
                  (cdc has no separate trigger — Debezium starts streaming on its own; this will report that
                  honestly)
                </span>
              ) : null}
            </Label>
          </div>
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
