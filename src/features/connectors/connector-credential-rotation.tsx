"use client"

import * as React from "react"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { useServiceAction } from "@/hooks/use-service"
import { isServiceError } from "@/services/errors"
import { connectorService } from "@/services"
import type {
  CredentialKind,
  CredentialSource,
  SecretSlot,
} from "@/services/contracts/connectors"
import { CREDENTIAL_KIND_OPTIONS } from "./credential-options"

/**
 * `PUT /api/connectors/{id}/secret` (`rotate_secret`,
 * `rust/crates/lakehouse-api/src/routes/connectors.rs`) probes the
 * CANDIDATE credential before writing anything (ADR 0002 Addendum 3). The
 * three outcomes this component must tell apart, by the response's HTTP
 * status rather than its message text (`ServiceError.status`,
 * `src/services/errors.ts`):
 *
 * - 200: the candidate probed clean and the connector's row now points at
 *   it. The response carries only `{ rotated, slot }`, never the derived
 *   name itself (`rotate_secret`'s own doc comment explains why: it is
 *   deterministic from the connector's id and the request's
 *   source/kind, so repeating it back adds nothing `derive_secret_ref`
 *   does not already let an operator recompute, and this component does
 *   not try — it says "rotated" and that the connector now dials the name
 *   derived from its own id, not a name it fabricates.
 * - 422: the probe could not verify the candidate (unsupported connector
 *   type) or it verified and failed — either way the OLD credential is
 *   still the one in use, nothing was written.
 * - 409: `swap_secret_ref`'s optimistic-concurrency check found the slot's
 *   ref had already changed since this handler read it — someone else
 *   rotated first.
 */
const SLOT_OPTIONS: { value: SecretSlot; label: string }[] = [
  { value: "primary", label: "Primary" },
  { value: "secondary", label: "Secondary" },
]

const SOURCE_OPTIONS: { value: CredentialSource; label: string }[] = [
  { value: "env", label: "Environment variable" },
  { value: "file", label: "Mounted file" },
]

export function ConnectorCredentialRotation({
  connectorId,
  onRotated,
}: {
  connectorId: string
  /** The drawer's own `state.reload()` — called after a successful
   * rotation so the detail panel re-fetches and shows the connector's
   * post-rotation state (e.g. a later `Test connection` dialing the new
   * ref). This component never reloads on its own: it does not own the
   * connector detail fetch, the drawer does. */
  onRotated: () => void
}) {
  const [slot, setSlot] = React.useState<SecretSlot>("primary")
  const [source, setSource] = React.useState<CredentialSource>("env")
  const [kind, setKind] = React.useState<CredentialKind>("password")

  const rotate = useServiceAction((signal) =>
    connectorService.rotateSecret(connectorId, { slot, source, kind }, signal)
  )

  async function handleRotate() {
    const result = await rotate.run()
    if (result) onRotated()
  }

  const error = rotate.status === "error" ? rotate.error : null
  // See this module's doc comment: 422 and 409 are told apart by
  // `ServiceError.status`, not by parsing `error.message` — the server's
  // message text is still shown, just not used to branch on.
  const isUnprocessable = isServiceError(error) && error.status === 422
  const isConflict = isServiceError(error) && error.status === 409

  return (
    <SectionCard
      title="Rotate credential"
      description="Replace one of this connector's credential slots with a new reference derived from its own id."
      size="sm"
    >
      <div className="space-y-3">
        <p className="text-xs text-muted-foreground">
          The new credential must already be provisioned under the derived name before you rotate
          — the server tests it first and never swaps to a name it could not dial (ADR 0002
          Addendum 3, <code className="rounded bg-muted px-1 py-0.5">docs/adr/0002-secretref-resolution.md</code>).
        </p>
        <div className="grid gap-3 sm:grid-cols-3">
          <div className="space-y-1.5">
            <Label htmlFor="rotate-slot">Slot</Label>
            <select
              id="rotate-slot"
              className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
              value={slot}
              onChange={(e) => setSlot(e.target.value as SecretSlot)}
            >
              {SLOT_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="rotate-source">Source</Label>
            <select
              id="rotate-source"
              className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
              value={source}
              onChange={(e) => setSource(e.target.value as CredentialSource)}
            >
              {SOURCE_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="rotate-kind">Kind</Label>
            <select
              id="rotate-kind"
              className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
              value={kind}
              onChange={(e) => setKind(e.target.value as CredentialKind)}
            >
              {CREDENTIAL_KIND_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>
        </div>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={rotate.status === "pending"}
          onClick={handleRotate}
        >
          {rotate.status === "pending" ? "Rotating…" : "Rotate credential"}
        </Button>
        {rotate.status === "success" && rotate.data ? (
          <p className="text-sm text-emerald-600 dark:text-emerald-400">
            Rotated — this connector now dials the {rotate.data.slot} credential derived from its
            own id.
          </p>
        ) : null}
        {isUnprocessable ? (
          <p className="text-sm text-destructive">
            Not rotated — {error?.message} The old credential is still in use.
          </p>
        ) : null}
        {isConflict ? (
          <p className="text-sm text-destructive">
            Not rotated — {error?.message} Reload this connector and try again.
          </p>
        ) : null}
        {error && !isUnprocessable && !isConflict ? (
          <p className="text-sm text-destructive">{error.message}</p>
        ) : null}
      </div>
    </SectionCard>
  )
}
