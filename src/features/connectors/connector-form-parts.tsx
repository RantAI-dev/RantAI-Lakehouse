"use client"

/**
 * Pieces the connector create and edit pages share, so the two cannot drift
 * apart: the adapter-specific connection form, the credential fields and
 * their validation, the tenant picker, and the small helpers both need.
 */

import * as React from "react"
import { PencilIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { CdcDialForm } from "@/features/connectors/dial-forms/cdc-dial-form"
import { CdcMongoDialForm } from "@/features/connectors/dial-forms/cdc-mongo-dial-form"
import { FilesDialForm } from "@/features/connectors/dial-forms/files-dial-form"
import { KafkaDialForm } from "@/features/connectors/dial-forms/kafka-dial-form"
import { MongoDialForm } from "@/features/connectors/dial-forms/mongo-dial-form"
import { OracleDialForm } from "@/features/connectors/dial-forms/oracle-dial-form"
import { RestDialForm } from "@/features/connectors/dial-forms/rest-dial-form"
import { SaasDialForm } from "@/features/connectors/dial-forms/saas-dial-form"
import { SftpDialForm } from "@/features/connectors/dial-forms/sftp-dial-form"
import { SheetsDialForm } from "@/features/connectors/dial-forms/sheets-dial-form"
import { SqlDialForm } from "@/features/connectors/dial-forms/sql-dial-form"
import { useAuth } from "@/features/auth/auth-provider"
import { cn } from "@/lib/utils"
import { SECRET_MANAGER_LABEL, referenceProblem } from "@/lib/connectors/secret-reference"
import { saasVendorById } from "@/lib/connectors/saas-vendors"
import type {
  CdcDial,
  CdcMongoDial,
  CredentialKind,
  CredentialSettings,
  CredentialSource,
  FilesDial,
  KafkaDial,
  MongoDial,
  RestDial,
  SaasDial,
  SftpDial,
  SheetsDial,
  SqlDial,
  SqlDriver,
} from "@/services/contracts/connectors"
import { CredentialValueInput, credentialValueProblem } from "./credential-value-input"
import { SecretReferenceField } from "./secret-reference-field"

/**
 * `CreateConnectorInput.host` still exists on the base connector row
 * (used by `connector_probe`'s own connectivity test, a DIFFERENT probe
 * than ingest discovery) -- derive it from whichever dial field is this
 * adapter's own "where do I connect" field, rather than asking for the
 * same value twice.
 */
export function hostFromDial(adapter: string | null, dial: Record<string, unknown> | null): string {
  if (!dial) return ""
  switch (adapter) {
    case "sql":
    case "cdc":
    case "sftp":
      // A MongoDB change-capture dial names `hosts`, not `host` (`SRC-4` slice 5).
      if (adapter === "cdc" && Array.isArray(dial.hosts)) {
        return typeof dial.hosts[0] === "string" ? (dial.hosts[0] as string) : ""
      }
      return typeof dial.host === "string" ? dial.host : ""
    case "files":
      return typeof dial.bucket === "string" ? dial.bucket : ""
    case "rest":
      return typeof dial.baseUrl === "string" ? dial.baseUrl : ""
    case "sheets":
      return typeof dial.spreadsheetId === "string" ? dial.spreadsheetId : ""
    case "saas":
      // The vendor's per-customer part of its address; the vendor table builds the rest (AD2).
      return typeof dial.account === "string" ? dial.account : ""
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

/**
 * One credential field the connection form asks for. `kind` only picks the
 * suffix of the reference name the server derives from the connector's id
 * (ADR 0002 Addendum 3); `label` is what the source actually expects in
 * that slot.
 */
export type CredentialSlot = {
  kind: CredentialKind
  label: string
  /** A PEM or JSON document: a textarea, and surrounding whitespace is
   * not significant. */
  multiline?: boolean
  /** Shown in clear (a username or client id is an identifier, not a
   * secret, even though it is stored like one). */
  plain?: boolean
  placeholder?: string
  /**
   * The value is not typed: it arrives when the person signs in at the
   * provider, from the connector page (`SRC-12` part C: the OAuth refresh
   * token). The form shows no field for it and sends no value for it.
   */
  signIn?: boolean
}

export type CredentialSlots = { primary: CredentialSlot; secondary: CredentialSlot | null }

const PASSWORD: CredentialSlot = { kind: "password", label: "Password" }

/**
 * The credential fields a connection needs, in slot order. Mirrors
 * `secret_field_names` (`rust/crates/lakehouse-store/src/ingest_spec.rs`,
 * `dagster/dispar_orchestrate/secret_map.py`): the primary slot is read as
 * the first named field and the secondary as the second, so a two-field
 * auth type (REST basic, OAuth2 client credentials, S3 keys) gets both
 * slots here, with the labels the adapter reads them as.
 *
 * Kafka `none` auth needs nothing, but the create request still requires
 * a primary kind; it seeds `password`, which is never provisioned or read
 * (see `usesNoCredential`).
 */
export function credentialSlotsFor(adapter: string | null, dial: Record<string, unknown> | null): CredentialSlots {
  const authType = (dial?.auth as { type?: unknown } | undefined)?.type
  switch (adapter) {
    case "files":
      return {
        primary: { kind: "access_key", label: "Access key ID", plain: true },
        secondary: { kind: "secret_key", label: "Secret access key" },
      }
    case "sftp":
      return authType === "public_key"
        ? {
            primary: {
              kind: "private_key",
              label: "Private key",
              multiline: true,
              placeholder: "-----BEGIN OPENSSH PRIVATE KEY-----",
            },
            secondary: null,
          }
        : { primary: PASSWORD, secondary: null }
    case "rest":
      switch (authType) {
        case "bearer":
          return { primary: { kind: "token", label: "Bearer token" }, secondary: null }
        case "basic":
          return {
            primary: { kind: "access_key", label: "Username", plain: true },
            secondary: PASSWORD,
          }
        case "oauth2_client_credentials":
          return {
            primary: { kind: "access_key", label: "Client ID", plain: true },
            secondary: { kind: "secret_key", label: "Client secret" },
          }
        case "oauth2_authorization_code":
          // The client id is part of the connection settings; the client secret
          // is typed, the refresh token comes from signing in.
          return {
            primary: { kind: "secret_key", label: "Client secret" },
            secondary: { kind: "token", label: "Refresh token", signIn: true },
          }
        default:
          return { primary: { kind: "api_key", label: "API key" }, secondary: null }
      }
    case "saas":
      return saasCredentialSlots(dial)
    case "sheets":
      return {
        primary: {
          kind: "password",
          label: "Service account JSON",
          multiline: true,
          placeholder: '{ "type": "service_account", ... }',
        },
        secondary: null,
      }
    default:
      return { primary: PASSWORD, secondary: null }
  }
}

/** How each named secret field of a vendor's integration credential is asked for. */
const SAAS_FIELD_SLOTS: Record<string, CredentialSlot> = {
  token: { kind: "token", label: "Access token" },
  email: { kind: "access_key", label: "Email address", plain: true },
  apiToken: { kind: "token", label: "API token" },
  username: { kind: "access_key", label: "Username", plain: true },
  password: PASSWORD,
  clientId: { kind: "access_key", label: "Client ID", plain: true },
  clientSecret: { kind: "secret_key", label: "Client secret" },
  privateKey: {
    kind: "private_key",
    label: "Private key",
    multiline: true,
    // PR #214 CI gitleaks: no PEM header literal here; a second one beside the OpenSSH placeholder made the text between them match as one key.
    placeholder: "Paste the PEM private key, starting with BEGIN PRIVATE KEY",
  },
  serviceAccountJson: {
    kind: "password",
    label: "Service account JSON",
    multiline: true,
    placeholder: '{ "type": "service_account", ... }',
  },
}

/**
 * The credential fields of a `saas` connector (`SRC-3`): a browser sign-in
 * asks for the client secret and gets the refresh token from signing in (as a
 * REST authorization-code connector does); an integration credential fills the
 * slots the vendor table names, in its order. Mirrors `secret_fields_for`
 * (`saas_vendors.rs`, `saas/vendors`).
 */
function saasCredentialSlots(dial: Record<string, unknown> | null): CredentialSlots {
  const vendor = saasVendorById(dial?.vendor)
  const authType = (dial?.auth as { type?: unknown } | undefined)?.type
  if (authType === "integration" && vendor?.integration) {
    const [first, second] = vendor.integration.secretFields.map((field) => SAAS_FIELD_SLOTS[field] ?? PASSWORD)
    // Slice 6: Workday asks for the integration system user as `user@tenant`, sent exactly as saved.
    const primary = vendor.id === "workday" && first ? { ...first, label: "Integration system user (as user@tenant)" } : (first ?? PASSWORD)
    return { primary, secondary: second ?? null }
  }
  return {
    primary: { kind: "secret_key", label: "Client secret" },
    secondary: { kind: "token", label: "Refresh token", signIn: true },
  }
}

/** A stable key for a slot layout, to reset typed values when it changes. */
export function credentialSlotsKey(slots: CredentialSlots): string {
  return [slots.primary.kind, slots.primary.label, slots.secondary?.kind, slots.secondary?.label].join("|")
}

/** Surrounding whitespace in a PEM or JSON document is not significant; a
 * password's is (and is refused rather than silently dropped). */
export function normalizedCredential(slot: CredentialSlot, value: string): string {
  return slot.multiline ? value.trim() : value
}

/**
 * The SQL driver a connector type's name already implies, so the SQL and
 * CDC forms do not ask for it a second time. `undefined` for a type that
 * names no single driver.
 */
export function driverForType(typeName: string | null): SqlDriver | undefined {
  // Oracle change capture only: the batch Oracle type has its own form (`DialFormFor`), so "Oracle" alone names no driver here.
  if (typeName === "Oracle CDC") return "oracle"
  switch (typeName?.replace(/ CDC$/, "")) {
    case "PostgreSQL":
      return "postgres"
    case "MySQL":
    case "MariaDB":
      return "mysql"
    case "SQL Server":
      return "mssql"
    default:
      return undefined
  }
}

export function DialFormFor({
  adapter,
  typeName,
  dial,
  onChange,
  oauthCallbackUrl,
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
  /** `CredentialSettings.oauthCallbackUrl`: shown by the REST form's OAuth sign-in fields. */
  oauthCallbackUrl?: string | null
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
          driver={driverForType(typeName)}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
        />
      )
    case "cdc":
      // MongoDB change capture has its own dial (hosts and a replica set, no host and port).
      if (typeName === "MongoDB CDC") {
        return (
          <CdcMongoDialForm
            value={dial as unknown as CdcMongoDial | null}
            onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
          />
        )
      }
      return (
        <CdcDialForm
          value={dial as unknown as CdcDial | null}
          driver={driverForType(typeName)}
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
          oauthCallbackUrl={oauthCallbackUrl}
        />
      )
    case "saas":
      return (
        <SaasDialForm
          typeName={typeName}
          value={dial as unknown as SaasDial | null}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
          oauthCallbackUrl={oauthCallbackUrl}
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

/** Whether a connection needs no credential at all (unauthenticated Kafka). */
export function usesNoCredential(adapter: string | null, dial: Record<string, unknown> | null): boolean {
  return adapter === "kafka" && (dial?.auth as { type?: unknown } | undefined)?.type === "none"
}

/**
 * Why the credential fields cannot be submitted yet, per slot (`null` =
 * fine). In `optional` mode (the edit page) blank means "keep the current
 * credential"; but a two-part credential is only ever replaced together,
 * so typing one half requires the other.
 */
export function credentialProblems({
  noCredential,
  optional,
  slots,
  primaryValue,
  secondaryValue,
  referenceManager = null,
}: {
  noCredential: boolean
  optional: boolean
  slots: CredentialSlots
  primaryValue: string
  secondaryValue: string
  /** Set while the form is in "Reference a secret" mode: the values are references. */
  referenceManager?: CredentialSettings["secretManager"]
}): { primary: string | null; secondary: string | null } {
  if (noCredential) return { primary: null, secondary: null }
  // A slot filled by signing in is never typed, so it is never a problem here.
  const typedSecondary = slots.secondary && !slots.secondary.signIn ? slots.secondary : null
  const nothingTyped = primaryValue === "" && (!typedSecondary || secondaryValue === "")
  if (optional && nothingTyped) return { primary: null, secondary: null }
  if (referenceManager) {
    const check = (reference: string) =>
      reference === "" ? "Required." : referenceProblem(referenceManager, reference)
    return {
      primary: check(primaryValue),
      secondary: typedSecondary ? check(secondaryValue) : null,
    }
  }
  return {
    primary: credentialValueProblem(normalizedCredential(slots.primary, primaryValue)),
    secondary: typedSecondary
      ? credentialValueProblem(normalizedCredential(typedSecondary, secondaryValue))
      : null,
  }
}

/**
 * The credential part of the Connection step. Write-only (ADR 0002
 * Addendum 4): the value is sent once and never shown again. Which fields
 * appear follows the connection's own auth type (`credentialSlotsFor`),
 * so there is no second "authenticate with" choice to keep in step.
 */
export function CredentialFields({
  noCredential,
  optional,
  credentialManaged,
  credentialSource,
  slots,
  primaryValue,
  onPrimaryValueChange,
  secondaryValue,
  onSecondaryValueChange,
  problems,
  settings = null,
  referenceMode = false,
  onReferenceModeChange,
}: {
  noCredential: boolean
  /** Edit page: blank keeps the current credential. */
  optional: boolean
  /** Edit page: whether the current credential is stored by lakehouse. */
  credentialManaged?: boolean
  /** Edit page: how the current credential is held (`SRC-12` decision 10). */
  credentialSource?: CredentialSource | null
  slots: CredentialSlots
  primaryValue: string
  onPrimaryValueChange: (value: string) => void
  secondaryValue: string
  onSecondaryValueChange: (value: string) => void
  problems: { primary: string | null; secondary: string | null }
  /** What this installation offers (`SRC-12` part B); `null` until loaded. */
  settings?: CredentialSettings | null
  /** Whether the credential is a reference to a secret in the secret manager. */
  referenceMode?: boolean
  onReferenceModeChange?: (referenceMode: boolean) => void
}) {
  if (noCredential) {
    return <p className="text-xs text-muted-foreground">This connection authenticates with no credential.</p>
  }
  const manager = settings?.secretManager ?? null
  // With a secret manager set up, the form offers "Reference a secret"; the
  // operator can turn typing a value off, which leaves only the reference.
  const showReference = manager !== null && referenceMode
  const typedSecondary = slots.secondary && !slots.secondary.signIn ? slots.secondary : null
  const pair = typedSecondary !== null && !slots.primary.multiline && !typedSecondary.multiline
  const field = (id: string, slot: CredentialSlot, value: string, onChange: (value: string) => void) =>
    showReference ? (
      <SecretReferenceField key={`${id}-${credentialSlotsKey(slots)}`} manager={manager} idPrefix={id} onChange={onChange} />
    ) : (
      <CredentialValueInput id={id} slot={slot} value={value} onChange={onChange} />
    )
  return (
    <div className={cn("grid gap-3", pair && !showReference && "sm:grid-cols-2")}>
      {manager !== null && onReferenceModeChange ? (
        <div className={cn("space-y-1", pair && !showReference && "sm:col-span-2")}>
          <div role="group" aria-label="How the credential is given" className="inline-flex gap-1">
            {settings?.typedCredentials ? (
              <Button
                type="button"
                size="sm"
                variant={referenceMode ? "outline" : "default"}
                aria-pressed={!referenceMode}
                onClick={() => onReferenceModeChange(false)}
              >
                Type a value
              </Button>
            ) : null}
            <Button
              type="button"
              size="sm"
              variant={referenceMode ? "default" : "outline"}
              aria-pressed={referenceMode}
              onClick={() => onReferenceModeChange(true)}
            >
              Reference a secret
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            Secret manager: {SECRET_MANAGER_LABEL[manager]}.
            {settings?.typedCredentials ? "" : " Typing a value is turned off on this installation."}
          </p>
        </div>
      ) : null}
      <Field
        label={showReference ? `${slots.primary.label} (reference)` : slots.primary.label}
        htmlFor={showReference ? undefined : "credential-primary"}
        error={primaryValue && problems.primary ? problems.primary : undefined}
      >
        {field("credential-primary", slots.primary, primaryValue, onPrimaryValueChange)}
      </Field>
      {typedSecondary ? (
        <Field
          label={showReference ? `${typedSecondary.label} (reference)` : typedSecondary.label}
          htmlFor={showReference ? undefined : "credential-secondary"}
          error={secondaryValue && problems.secondary ? problems.secondary : undefined}
        >
          {field("credential-secondary", typedSecondary, secondaryValue, onSecondaryValueChange)}
        </Field>
      ) : null}
      {slots.secondary?.signIn ? (
        <p className="text-xs text-muted-foreground">
          The {slots.secondary.label.toLowerCase()} is not entered here: you get it by signing in at the
          provider, from the connector page, once the connector is saved.
        </p>
      ) : null}
      <p className={cn("text-xs text-muted-foreground", pair && "sm:col-span-2")}>
        {showReference
          ? "Only the reference is stored. The product reads the secret from the secret manager at every test and every run, after checking now that it can read it and that the connection works. Changing the secret there takes effect at the next run."
          : optional
          ? credentialManaged
            ? "Leave blank to keep the current credential. A new one is tested against the source before it replaces the old."
            : credentialSource === "env"
              ? "This connector's credential is an environment variable on the server, read when the API and the orchestrator start: if an operator changes it, they must restart both. Leave blank to keep it, or enter one to have lakehouse store it instead."
              : "This connector's credential is provisioned by an operator on the server. Leave blank to keep it, or enter one to have lakehouse store it instead."
          : "Stored by lakehouse when you create the connector, and never shown again. You can replace it later from the connector's edit page."}
      </p>
    </div>
  )
}

/**
 * The tenant a connector belongs to, picked from the tenants the current
 * user actually belongs to — not free text. A connector's tenant decides
 * who can see it in the connector list.
 */
export function TenantField({
  value,
  onChange,
  disabled,
  hint,
}: {
  value: string
  onChange: (tenantId: string) => void
  disabled?: boolean
  hint?: string
}) {
  const { user } = useAuth()
  const tenants = user?.tenants ?? []
  return (
    <Field label="Tenant" htmlFor="connector-tenant">
      {tenants.length === 0 ? (
        <p className="text-xs text-destructive">
          Your account belongs to no tenant, so a connector you create would not appear in any connector
          list. Ask an administrator to add you to a tenant.
        </p>
      ) : (
        <select
          id="connector-tenant"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
        >
          {value === "" ? <option value="">Unassigned</option> : null}
          {tenants.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
      )}
      {hint ? <p className="text-xs text-muted-foreground">{hint}</p> : null}
    </Field>
  )
}

/** A review section's way back to the step it summarizes. */
export function EditStepButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={`Edit ${label}`}
      className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium text-primary outline-none hover:bg-primary/10 focus-visible:ring-[3px] focus-visible:ring-ring/50"
    >
      <PencilIcon className="size-3" />
      Edit
    </button>
  )
}

/** A titled part of a wizard step: an icon, a title and one line on what it is for. */
export function StepSectionHeading({
  icon,
  title,
  description,
}: {
  icon: React.ReactNode
  title: string
  description: string
}) {
  return (
    <div className="flex items-start gap-2.5">
      <span className="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
        {icon}
      </span>
      <div className="min-w-0">
        <h3 className="text-sm font-medium">{title}</h3>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
    </div>
  )
}

export function Field({
  label,
  children,
  className,
  htmlFor,
  error,
}: {
  label: string
  children: React.ReactNode
  className?: string
  htmlFor?: string
  error?: string
}) {
  return (
    <div className={cn("space-y-1.5", className)}>
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  )
}
