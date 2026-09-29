"use client"

import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import type { CredentialKind } from "@/services/contracts/connectors"
import type { CredentialSlot } from "./connector-form-parts"
import { CREDENTIAL_KIND_OPTIONS } from "./credential-options"

/** The display label for a credential kind, e.g. `"Password"`. */
export function credentialKindLabel(kind: CredentialKind): string {
  return CREDENTIAL_KIND_OPTIONS.find((o) => o.value === kind)?.label ?? kind
}

/**
 * Why a credential value would be refused server-side, or `null` when it
 * is usable. Mirrors `validate_secret_value`
 * (`rust/crates/lakehouse-api/src/connector_secret_store.rs`) so the form
 * can say so before submitting: both resolvers read a stored credential
 * back trimmed, so leading/trailing whitespace would make a saved password
 * silently never match.
 */
export function credentialValueProblem(value: string): string | null {
  if (value.trim() === "") return "Required."
  if (value.trim() !== value) return "Must not start or end with a space."
  return null
}

/**
 * A write-only credential field (ADR 0002 Addendum 4). The value goes to
 * the server once and is never shown again, so a secret is a password
 * input (a textarea for a PEM or JSON document) with the browser's own
 * autofill of saved passwords suppressed — the stored login for this
 * console is not the source's credential. An identifier half of a pair
 * (a username, a client id) is shown in clear.
 */
export function CredentialValueInput({
  id,
  slot,
  value,
  onChange,
}: {
  id: string
  slot: CredentialSlot
  value: string
  onChange: (next: string) => void
}) {
  if (slot.multiline) {
    return (
      <Textarea
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={slot.placeholder}
        autoComplete="off"
        spellCheck={false}
        className="font-mono text-xs"
        rows={5}
      />
    )
  }
  return (
    <Input
      id={id}
      type={slot.plain ? "text" : "password"}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={slot.placeholder}
      autoComplete={slot.plain ? "off" : "new-password"}
      spellCheck={false}
    />
  )
}
