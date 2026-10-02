"use client"

import * as React from "react"
import Link from "next/link"
import { RefreshCwIcon } from "lucide-react"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatDateTime } from "@/lib/format"
import { fmtMeasured } from "@/lib/measured"
import { goldService } from "@/services"
import type { GoldExportRun, GoldPublication, GoldReadBack } from "@/services/contracts/gold"

function trimmedMart(assetId: string): string {
  return assetId.startsWith("serving.") ? assetId.slice("serving.".length) : assetId
}

function freshnessLine(pub: GoldPublication): string {
  if (pub.lastExportedAt === null) return "Never published"
  if (pub.lastChangedAt === null) return "Not measured"
  if (pub.lastChangedAt < pub.lastExportedAt) return "Up to date"
  return "Out of date"
}

function LastFiveRuns({ mart }: { mart: string }) {
  const state = useService<GoldExportRun[]>(
    (signal) => goldService.listExportRuns(mart, signal),
    [mart]
  )
  if (state.status === "loading") return <LoadingSkeleton rows={3} />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  const runs = state.data.slice(0, 5)
  if (runs.length === 0) return <p className="text-sm text-muted-foreground">No publish runs yet.</p>
  return (
    <Table>
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          <TableHead>Status</TableHead>
          <TableHead>Time</TableHead>
          <TableHead>Rows</TableHead>
          <TableHead>Snapshot</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {runs.map((r) => (
          <TableRow key={r.id}>
            <TableCell className={r.status === "failed" ? "text-destructive" : ""}>
              {r.status}
            </TableCell>
            <TableCell className="text-muted-foreground">
              {formatDateTime(r.startedAt)}
            </TableCell>
            <TableCell>{fmtMeasured(r.rowsExported)}</TableCell>
            <TableCell>
              {fmtMeasured(r.snapshotId, (v) => String(v))}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}

export function OpenFormatCard({ assetId }: { assetId: string }) {
  const mart = trimmedMart(assetId)
  const pub = useService<GoldPublication>(
    (signal) => goldService.getPublication(mart, signal),
    [mart]
  )
  const lastExport = useService<GoldReadBack | null>(async (signal) => {
    try {
      return await goldService.getLastExport(mart, signal)
    } catch {
      return null
    }
  }, [mart])
  const toggle = useServiceAction(
    (signal, enabled: boolean) => goldService.setPublication(mart, enabled, signal)
  )
  const [toggleError, setToggleError] = React.useState<string | null>(null)
  const exportNow = useServiceAction((signal) => goldService.triggerExport(mart, signal))

  async function handleToggle(checked: boolean) {
    setToggleError(null)
    const result = await toggle.run(checked)
    if (result === null) {
      setToggleError(toggle.error?.message ?? "Failed to toggle publishing.")
    } else {
      pub.reload()
    }
  }

  if (pub.status === "loading") return <SectionCard title="Publish in open format (Iceberg)"><LoadingSkeleton rows={2} /></SectionCard>
  if (pub.status === "error") return <SectionCard title="Publish in open format (Iceberg)"><ErrorState error={pub.error} onRetry={pub.reload} /></SectionCard>

  const p = pub.data
  const enabled = p.enabled

  const canEdit = p.canEdit
  const stateLabel = enabled ? freshnessLine(p) : "Off"

  return (
    <SectionCard
      title="Publish in open format (Iceberg)"
      description="A copy in open Iceberg format that outside tools can read. Publishing adds one full copy each time; old copies stay."
      action={
        <label className="flex items-center gap-2" data-testid="open-format-switch">
          <input
            type="checkbox"
            role="switch"
            checked={enabled}
            disabled={!canEdit || toggle.status === "pending"}
            aria-label="Publish in open format (Iceberg)"
            onChange={(e) => {
              void handleToggle(e.currentTarget.checked)
            }}
            className="peer relative inline-flex h-[18px] w-[32px] shrink-0 cursor-pointer items-center rounded-full border border-transparent transition-all outline-none after:absolute after:-inset-x-3 after:-inset-y-2 focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 data-disabled:cursor-not-allowed data-disabled:opacity-50 bg-input data-checked:bg-primary"
          />
          <span className="text-sm text-muted-foreground">
            {toggle.status === "pending"
              ? "Saving…"
              : enabled
                ? "On"
                : "Off"}
          </span>
        </label>
      }
    >
      <div className="flex flex-col gap-3">
        {!canEdit && (
          <p className="text-xs text-muted-foreground">
            You do not hold the gold:export permission. Only a Platform Admin may switch publishing on or off.
          </p>
        )}
        {toggleError && (
          <p role="alert" className="text-sm text-destructive">{toggleError}</p>
        )}
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <span>Status:</span>
          <span className={
            stateLabel === "Up to date"
              ? "font-medium text-green-600 dark:text-green-400"
              : stateLabel === "Out of date"
                ? "font-medium text-amber-600 dark:text-amber-400"
                : stateLabel === "Not measured"
                  ? "font-medium text-muted-foreground"
                  : stateLabel === "Never published"
                    ? "font-medium text-muted-foreground"
                    : "font-medium text-muted-foreground"
          }>
            {stateLabel}
          </span>
        </div>
        {enabled && lastExport.status === "success" && lastExport.data && (
          <>
            <dl className="grid grid-cols-3 gap-1 text-sm">
              <dt className="text-muted-foreground">Last published</dt>
              <dd className="col-span-2">{lastExport.data.exportedAt ? formatDateTime(lastExport.data.exportedAt) : "—"}</dd>
              <dt className="text-muted-foreground">Snapshot</dt>
              <dd className="col-span-2 font-mono text-xs">
                {fmtMeasured(lastExport.data.snapshotId, (v) => String(v))}
              </dd>
            </dl>
          </>
        )}
        {enabled && (
          <>
            <div className="flex flex-col items-start gap-1">
              <Button
                size="sm"
                variant="outline"
                disabled={exportNow.status === "pending"}
                onClick={() => {
                  void exportNow.run().then((result) => {
                    if (result) {
                      lastExport.reload()
                      pub.reload()
                    }
                  })
                }}
              >
                <RefreshCwIcon data-icon="inline-start" />
                {exportNow.status === "pending" ? "Publishing…" : "Publish now"}
              </Button>
              {exportNow.status === "error" && (
                <p role="alert" className="text-xs text-destructive">
                  {exportNow.error.message}
                </p>
              )}
            </div>
            <div>
              <h4 className="mb-1 text-sm font-medium">Last 5 publish runs</h4>
              <LastFiveRuns mart={mart} />
            </div>
            <Link
              href="/gold-exports"
              className="self-start text-sm text-primary hover:underline"
            >
              All published marts
            </Link>
          </>
        )}
      </div>
    </SectionCard>
  )
}