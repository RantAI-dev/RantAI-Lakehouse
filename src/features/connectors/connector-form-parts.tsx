"use client"

/**
 * Pieces the connector create and edit pages share, so the two cannot drift
 * apart: the adapter-specific connection form, the credential fields and
 * their validation, the tenant picker, and the small helpers both need.
 */

import * as React from "react"
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
import { useAuth } from "@/features/auth/auth-provider"
import { cn } from "@/lib/utils"
import type {
  CdcDial,
  CredentialKind,
  FilesDial,
  KafkaDial,
  MongoDial,
  RestDial,
  SftpDial,
  SheetsDial,
  SqlDial,
  SqlDriver,
} from "@/services/contracts/connectors"
import { CredentialValueInput, credentialValueProblem } from "./credential-value-input"

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
        default:
          return { primary: { kind: "api_key", label: "API key" }, secondary: null }
      }
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
          driver={driverForType(typeName)}
          onChange={(next) => onChange(next as unknown as Record<string, unknown>)}
        />
      )
    case "cdc":
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
}: {
  noCredential: boolean
  optional: boolean
  slots: CredentialSlots
  primaryValue: string
  secondaryValue: string
}): { primary: string | null; secondary: string | null } {
  if (noCredential) return { primary: null, secondary: null }
  const nothingTyped = primaryValue === "" && (!slots.secondary || secondaryValue === "")
  if (optional && nothingTyped) return { primary: null, secondary: null }
  return {
    primary: credentialValueProblem(normalizedCredential(slots.primary, primaryValue)),
    secondary: slots.secondary
      ? credentialValueProblem(normalizedCredential(slots.secondary, secondaryValue))
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
  slots,
  primaryValue,
  onPrimaryValueChange,
  secondaryValue,
  onSecondaryValueChange,
  problems,
}: {
  noCredential: boolean
  /** Edit page: blank keeps the current credential. */
  optional: boolean
  /** Edit page: whether the current credential is stored by lakehouse. */
  credentialManaged?: boolean
  slots: CredentialSlots
  primaryValue: string
  onPrimaryValueChange: (value: string) => void
  secondaryValue: string
  onSecondaryValueChange: (value: string) => void
  problems: { primary: string | null; secondary: string | null }
}) {
  if (noCredential) {
    return <p className="text-xs text-muted-foreground">This connection authenticates with no credential.</p>
  }
  const pair = slots.secondary !== null && !slots.primary.multiline && !slots.secondary.multiline
  return (
    <div className={cn("grid gap-3", pair && "sm:grid-cols-2")}>
      <Field
        label={slots.primary.label}
        htmlFor="credential-primary"
        error={primaryValue && problems.primary ? problems.primary : undefined}
      >
        <CredentialValueInput
          id="credential-primary"
          slot={slots.primary}
          value={primaryValue}
          onChange={onPrimaryValueChange}
        />
      </Field>
      {slots.secondary ? (
        <Field
          label={slots.secondary.label}
          htmlFor="credential-secondary"
          error={secondaryValue && problems.secondary ? problems.secondary : undefined}
        >
          <CredentialValueInput
            id="credential-secondary"
            slot={slots.secondary}
            value={secondaryValue}
            onChange={onSecondaryValueChange}
          />
        </Field>
      ) : null}
      <p className={cn("text-xs text-muted-foreground", pair && "sm:col-span-2")}>
        {optional
          ? credentialManaged
            ? "Leave blank to keep the current credential. A new one is tested against the source before it replaces the old."
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
