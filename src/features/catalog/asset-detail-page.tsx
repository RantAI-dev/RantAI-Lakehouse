"use client"

import * as React from "react"
import Link from "next/link"
import { useParams } from "next/navigation"
import { EntityHeader } from "@/components/patterns/page-header"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import {
  ClassificationBadge,
  HealthBadge,
  TierBadge,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { requestableAccessPermissions } from "@/lib/access-requests"
import { formatBytes, formatCompactNumber, formatRelativeTime } from "@/lib/format"
import { fmtMeasured } from "@/lib/measured"
import { DATA_LAYER_LABEL, ENGINE_CATEGORY_LABEL } from "@/lib/status"
import { assetService } from "@/services"
import { ASSET_TYPE_LABEL, type RequestAccessInput } from "@/services/contracts/assets"
import { AssetDetailTabs } from "./asset-detail-tabs"

/**
 * Candidate permissions the "Request access" dialog offers. Not every
 * governed permission — narrowed to the two a catalog viewer would
 * plausibly need: `catalog:write` (the Phase E acceptance criterion the
 * WS7 plan names) and `lineage:read` (gates this same page's lineage
 * tab). `requestableAccessPermissions` (`@/lib/access-requests`) further
 * excludes whichever of these the signed-in principal already holds — a
 * permission already granted is never offered, since requesting it 400s
 * server-side (`routes::catalog::access_request`'s own check).
 */
const REQUESTABLE_PERMISSIONS = ["catalog:write", "lineage:read"] as const

/** Asset detail with schema, freshness, lineage hops, policies, and snapshots. */
export function AssetDetailPage() {
  const params = useParams<{ assetId: string }>()
  const state = useService(
    (s) => assetService.getAsset(params.assetId, s),
    [params.assetId]
  )
  const { hasPermission } = useAuth()
  const [open, setOpen] = React.useState(false)
  const [permission, setPermission] = React.useState("")
  const [reason, setReason] = React.useState("")
  const [submitted, setSubmitted] = React.useState(false)
  const request = useServiceAction((signal, catalogId: string, input: RequestAccessInput) => {
    // `requestAccess` is optional on `AssetService` only so the dead
    // `mock/assets.ts` fixture (never wired to `assetService`, see that
    // contract's own doc comment) still satisfies the interface — the
    // real (ClickHouse-backed) client this page always talks to
    // implements it. Fail closed rather than silently no-op if that
    // assumption is ever wrong.
    if (!assetService.requestAccess) {
      return Promise.reject(new Error("This deployment cannot request catalog access."))
    }
    return assetService.requestAccess(catalogId, input, signal)
  })

  const missingPermissions = requestableAccessPermissions(
    REQUESTABLE_PERMISSIONS,
    hasPermission
  )

  function closeDialog() {
    setOpen(false)
    setPermission("")
    setReason("")
    setSubmitted(false)
    request.reset()
  }

  async function submitRequest(catalogId: string) {
    if (!permission) return
    // `requestAccess` resolves with no value on success; `useServiceAction`
    // resolves to `null` only on failure (the `ServiceError` is surfaced
    // via `request.error` below), so any non-`null` result here means the
    // POST 2xx'd.
    const ok = await request.run(catalogId, { permission, reason: reason.trim() })
    if (ok !== null) setSubmitted(true)
  }

  if (state.status === "loading") return <LoadingSkeleton rows={8} />
  if (state.status === "error")
    return <ErrorState error={state.error} onRetry={state.reload} />
  const a = state.data

  return (
    <div className="flex flex-col gap-3">
      <EntityHeader
        className="pb-3"
        eyebrow={<Link href="/data" className="hover:underline">Data Explorer</Link>}
        title={a.name}
        titleAccessory={
          <>
            <TierBadge tier={a.tier} />
            <ClassificationBadge classification={a.classification} />
            <HealthBadge health={a.health} />
            {missingPermissions.length > 0 ? (
              <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
                Request access
              </Button>
            ) : null}
          </>
        }
        description={a.description}
      />
      <MetadataList
        density="compact"
        columns={3}
        items={[
          { label: "Namespace", value: <span className="font-mono text-xs">{a.namespace}</span> },
          { label: "Type", value: ASSET_TYPE_LABEL[a.type] },
          { label: "Layer", value: DATA_LAYER_LABEL[a.layer] },
          { label: "Format", value: a.format },
          { label: "Engine", value: ENGINE_CATEGORY_LABEL[a.engine] },
          { label: "Rows", value: fmtMeasured(a.rows, formatCompactNumber) },
          { label: "Size", value: fmtMeasured(a.sizeBytes, formatBytes) },
          { label: "Owner", value: a.owner },
          { label: "Residency", value: a.residency },
          {
            label: "Freshness",
            value: <FreshnessIndicator lagSeconds={a.freshnessLagSeconds} />,
          },
          {
            label: "Updated",
            value: a.lastUpdated === null ? "—" : formatRelativeTime(a.lastUpdated),
          },
        ]}
      />
      <AssetDetailTabs asset={a} />

      <Dialog open={open} onOpenChange={(next) => (next ? setOpen(true) : closeDialog())}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Request access</DialogTitle>
            <DialogDescription>
              Ask a Governance Admin for a permission you don&apos;t hold on{" "}
              {a.name}. The request is pending until a different person
              decides it — see the Approvals inbox.
            </DialogDescription>
          </DialogHeader>
          {submitted ? (
            <p className="text-sm text-muted-foreground">
              Request sent — it is now pending in the Approvals inbox.
            </p>
          ) : (
            <div className="grid gap-3">
              <div className="grid gap-1.5">
                <Label>Permission</Label>
                <Select value={permission} onValueChange={(v) => setPermission(v ?? "")}>
                  <SelectTrigger>
                    <SelectValue placeholder="pick a permission" />
                  </SelectTrigger>
                  <SelectContent>
                    {missingPermissions.map((p) => (
                      <SelectItem key={p} value={p}>
                        {p}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1.5">
                <Label>Reason</Label>
                <Textarea
                  value={reason}
                  onChange={(e) => setReason(e.target.value)}
                  placeholder="Why do you need this?"
                  rows={3}
                />
              </div>
              {request.error ? (
                <p className="text-sm text-destructive">{request.error.message}</p>
              ) : null}
            </div>
          )}
          <DialogFooter>
            <DialogClose render={<Button variant="ghost" size="sm" />}>
              {submitted ? "Close" : "Cancel"}
            </DialogClose>
            {submitted ? null : (
              <Button
                size="sm"
                onClick={() => void submitRequest(a.id)}
                disabled={!permission || request.status === "pending"}
              >
                {request.status === "pending" ? "Requesting…" : "Request"}
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
