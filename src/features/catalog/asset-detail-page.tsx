"use client"

import * as React from "react"
import { useParams } from "next/navigation"
import { ErrorState } from "@/components/patterns/page-states"
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
import { assetService } from "@/services"
import type { AssetDetail, RequestAccessInput } from "@/services/contracts/assets"
import { AssetDetailTabsView } from "./asset-detail-tabs"
import { AssetHeader, AssetPageSkeleton, TrustStrip } from "./asset-header"
import { useIcebergTable } from "./asset-storage"
import type { MeaningState } from "./asset-meaning"
import { useMeanings } from "./asset-column-data"

/**
 * Candidate permissions the "Request access" dialog offers. Not every
 * governed permission — narrowed to the three a catalog viewer would
 * plausibly need: `catalog:write` (the Phase E acceptance criterion the
 * WS7 plan names), `lineage:read` (gates this same page's lineage
 * tab), and `query:read` (gates this page's sample rows and column
 * profile). `requestableAccessPermissions` (`@/lib/access-requests`) further
 * excludes whichever of these the signed-in principal already holds — a
 * permission already granted is never offered, since requesting it 400s
 * server-side (`routes::catalog::access_request`'s own check).
 */
const REQUESTABLE_PERMISSIONS = ["catalog:write", "lineage:read", "query:read"] as const

/**
 * An asset's page (`UX-8`, mockup A): the header with its ⋯ and Query, one
 * strip of trust facts, then the tabs, with an Overview that reads like a
 * readme.
 */
export function AssetDetailPage() {
  const params = useParams<{ assetId: string }>()
  // WEB-3 S3-D1: a column's sensitivity and masks change from inside the
  // opened column; a reload that swapped the page for a skeleton would close
  // it. Keep the asset on screen while the same asset reloads.
  const state = useService(
    (s) => assetService.getAsset(params.assetId, s),
    [params.assetId],
    { keepDataOnReload: true }
  )
  const { hasPermission } = useAuth()
  // Read here, not in the Schema tab, so the header pill and the tab share one request.
  const meanings = useMeanings(state.status === "success" ? state.data : null)
  const [open, setOpen] = React.useState(false)
  const [permission, setPermission] = React.useState("")
  const [reason, setReason] = React.useState("")
  const [submitted, setSubmitted] = React.useState(false)
  const request = useServiceAction((signal, catalogId: string, input: RequestAccessInput) => {
    // PR #106 review R2 SHOULD-FIX 6: requestAccess is required on AssetService
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

  if (state.status === "loading") return <AssetPageSkeleton />
  if (state.status === "error")
    return <ErrorState error={state.error} onRetry={state.reload} />
  const a = state.data

  return (
    <div className="flex flex-col gap-4">
      <AssetBody
        asset={a}
        meanings={meanings}
        missingPermissions={missingPermissions}
        onRequestAccess={() => setOpen(true)}
        onAssetChanged={state.reload}
      />

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
              {/* UX-4: permission is required and the reason is optional, so
                  the star stays; two inputs are too few for a legend. */}
              <div className="grid gap-1.5">
                <Label htmlFor="access-permission" required>Permission</Label>
                <Select value={permission} onValueChange={(v) => setPermission(v ?? "")}>
                  <SelectTrigger id="access-permission" aria-required>
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
                <Label htmlFor="access-reason">Reason <span className="font-normal text-muted-foreground">(optional)</span></Label>
                <Textarea
                  id="access-reason"
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

/**
 * The header, the trust strip and the tabs, which share one read of the
 * asset's Iceberg table: the header needs it for the Query link and the
 * Drop table item, the tabs for their own parts, and a second read would be
 * a second answer (`DATA-54`).
 *
 * `UX-8` (mockup A): the facts strip that sat here (namespace, type, layer,
 * format, engine, rows, size, owner, residency, freshness, updated) is gone;
 * each fact is said once — the address, layer and type in the header,
 * freshness and updated in the trust strip, the rest in the Overview's
 * Details.
 */
function AssetBody({
  asset: a,
  meanings,
  missingPermissions,
  onRequestAccess,
  onAssetChanged,
}: {
  asset: AssetDetail
  meanings: MeaningState
  missingPermissions: readonly string[]
  onRequestAccess: () => void
  onAssetChanged: () => void
}) {
  const iceberg = useIcebergTable(a)
  return (
    <>
      <AssetHeader
        asset={a}
        iceberg={iceberg}
        meanings={meanings}
        missingPermissions={missingPermissions}
        onRequestAccess={onRequestAccess}
        onChanged={onAssetChanged}
      />
      <TrustStrip asset={a} />
      <AssetDetailTabsView
        asset={a}
        iceberg={iceberg}
        onAssetChanged={onAssetChanged}
        meanings={meanings}
      />
    </>
  )
}
