"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  REFERENCE_PARTS,
  composeReference,
  type ReferenceParts,
  type SecretManager,
} from "@/lib/connectors/secret-reference"

/**
 * The fields of a reference to a secret in the installation's secret manager
 * (`SRC-12` part B, decision 2): the secret's path or name, and the key
 * inside it where the manager has keys. A reference names a place, never a
 * value, so these are plain text inputs and are shown back.
 *
 * It reports the reference string the API stores (`onChange`), or `""` while
 * nothing is typed. The server reads the reference once and tests the
 * connection before saving; it is the authority on whether it is valid.
 */
export function SecretReferenceField({
  manager,
  idPrefix,
  onChange,
}: {
  manager: SecretManager
  idPrefix: string
  onChange: (reference: string) => void
}) {
  const [parts, setParts] = React.useState<ReferenceParts>({})
  const update = (key: string, value: string) => {
    const next = { ...parts, [key]: value }
    setParts(next)
    onChange(Object.values(next).some((v) => v.trim() !== "") ? composeReference(manager, next) : "")
  }
  return (
    <div className="grid gap-2 sm:grid-cols-2">
      {REFERENCE_PARTS[manager].map((part) => (
        <div key={part.key} className="space-y-1">
          <Label htmlFor={`${idPrefix}-${part.key}`} className="text-xs text-muted-foreground">
            {part.label}
            {part.optional ? " (optional)" : ""}
          </Label>
          <Input
            id={`${idPrefix}-${part.key}`}
            value={parts[part.key] ?? ""}
            onChange={(e) => update(part.key, e.target.value)}
            placeholder={part.placeholder}
            autoComplete="off"
            spellCheck={false}
          />
        </div>
      ))}
    </div>
  )
}
