"use client"

import * as React from "react"
import { Trash2 } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatDate } from "@/lib/format"
import { notifyError, notifySuccess, withNotify } from "@/lib/notify"
import { assetService } from "@/services"
import type { AssetDetail, AssetGrant } from "@/services/contracts/assets"

/** The permissions a grant from this page may carry: the three a user can request. */
const GRANTABLE = ["catalog:write", "lineage:read", "query:read"] as const

const selectClassName = "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

/** The server's sentence, without the `permission_denied: ` its 403 body carries. */
function sentence(message: string): string {
  return message.replace(/^permission_denied:\s*/, "")
}

/**
 * The Access tab's Grants card (`DATA-15` F3), for a holder of
 * `access:approve`: the grants made from this asset, a form to give a user a
 * permission by e-mail, and Revoke.
 *
 * The product's permissions are global, so the card says what a grant is:
 * the permission for every table, for 30 days. The asset is only where the
 * grant was made, so it can be listed and revoked from here. The server's
 * refusals (unknown e-mail, oneself, a permission already held) are shown
 * in its own words under the form; nothing is toasted over them.
 */
export function AssetGrants({ asset: a }: { asset: AssetDetail }) {
  const { hasPermission } = useAuth()
  const allowed = hasPermission("access:approve")
  const grants = useService(
    (s) => (allowed ? assetService.listGrants(a.id, s) : Promise.resolve(null)),
    [a.id, allowed]
  )
  const [email, setEmail] = React.useState("")
  const [permission, setPermission] = React.useState<string>(GRANTABLE[0])
  const [revoking, setRevoking] = React.useState<AssetGrant | null>(null)
  const grant = useServiceAction((signal, input: { email: string; permission: string }) => {
    return assetService.grantAccess(a.id, input, signal)
  })
  // PR #106 review R1 SHOULD-FIX DATA-15 1: withNotify delivers error toast with real error
  const revoke = useServiceAction(
    withNotify(
      { success: "Grant revoked", error: "Failed to revoke the grant" },
      (signal, grantId: string) => assetService.revokeGrant(a.id, grantId, signal)
    )
  )

  if (!allowed) return null

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    if ((await grant.run({ email: email.trim(), permission })) === null) return
    notifySuccess("Access granted")
    setEmail("")
    grants.reload()
  }

  async function confirmRevoke() {
    if (!revoking) return
    // PR #106 review R1 SHOULD-FIX DATA-15 1
    if ((await revoke.run(revoking.id)) === null) return
    setRevoking(null)
    grants.reload()
  }

  const list = grants.status === "success" ? (grants.data ?? []) : []

  return (
    <SectionCard
      size="sm"
      title="Grants"
      description="A grant gives this permission for every table, for 30 days. The person loses it at their next request after you revoke it."
    >
      {grants.status === "loading" ? (
        <Skeleton className="h-16 w-full" />
      ) : grants.status === "error" ? (
        <ErrorState error={grants.error} onRetry={grants.reload} />
      ) : list.length === 0 ? (
        <p className="text-sm text-muted-foreground">No grant has been made from this asset.</p>
      ) : (
        <ul className="divide-y divide-border text-sm">
          {list.map((g) => (
            <li key={g.id} className="flex flex-wrap items-center gap-2 py-1.5">
              <span className="min-w-0">
                <span className="font-medium">{g.user}</span>{" "}
                <span className="text-xs text-muted-foreground">{g.userEmail}</span>
              </span>
              <span className="font-mono text-xs text-muted-foreground">{g.permission}</span>
              <Pill tone="neutral">{g.via === "direct" ? "Granted here" : "Approved request"}</Pill>
              <span className="ml-auto text-xs text-muted-foreground">
                {`${g.grantedBy ? `by ${g.grantedBy}, ` : ""}until ${formatDate(g.expiresAt)}`}
              </span>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={`Revoke ${g.permission} from ${g.user}`}
                onClick={() => setRevoking(g)}
              >
                <Trash2 />
              </Button>
            </li>
          ))}
        </ul>
      )}
      <form onSubmit={(e) => void submit(e)} className="mt-3 flex flex-wrap items-end gap-2">
        <div className="min-w-48 flex-1">
          <Label htmlFor="grant-email" className="mb-1 block text-xs">
            E-mail of the user
          </Label>
          <Input
            id="grant-email"
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="name@company.example"
          />
        </div>
        <div className="w-44">
          <Label htmlFor="grant-permission" className="mb-1 block text-xs">
            Permission
          </Label>
          <select
            id="grant-permission"
            className={selectClassName}
            value={permission}
            onChange={(e) => setPermission(e.target.value)}
          >
            {GRANTABLE.map((p) => (
              <option key={p} value={p}>
                {p}
              </option>
            ))}
          </select>
        </div>
        <Button type="submit" size="sm" disabled={email.trim() === "" || grant.status === "pending"}>
          Grant
        </Button>
      </form>
      {grant.status === "error" ? (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {sentence(grant.error.message)}
        </p>
      ) : null}
      <ConfirmActionDialog
        open={revoking !== null}
        onOpenChange={(open) => (open ? undefined : setRevoking(null))}
        title="Revoke grant"
        description={`Revoke ${revoking?.permission ?? ""} from ${revoking?.user ?? ""}?`}
        impact="They lose this permission for every table at their next request, unless a role of theirs gives it."
        confirmLabel="Revoke"
        confirming={revoke.status === "pending"}
        destructive
        onConfirm={() => void confirmRevoke()}
      />
    </SectionCard>
  )
}
