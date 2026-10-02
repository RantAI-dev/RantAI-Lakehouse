"use client"

import * as React from "react"
import Link from "next/link"
import { RefreshCwIcon } from "lucide-react"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
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
import { freshnessLine } from "@/lib/gold-freshness"
import { fmtMeasured } from "@/lib/measured"
import { goldService } from "@/services"
import type { GoldExportRun, GoldPublication, GoldReadBack } from "@/services/contracts/gold"

function trimmedMart(assetId: string): string {
  return assetId.startsWith("serving.") ? assetId.slice("serving.".length) : assetId
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
              {r.status === "failed" && r.error && (
                <div className="text-xs text-destructive/80">{r.error}</div>
              )}
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

const stateLabelClass = (label: string): string => {
  if (label === "Up to date") return "font-medium text-green-600 dark:text-green-400"
  if (label === "Out of date") return "font-medium text-amber-600 dark:text-amber-400"
  return "font-medium text-muted-foreground"
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
  const exportNow = useServiceAction((signal) => goldService.triggerExport(mart, signal))

  async function handleToggle(checked: boolean) {
    const result = await toggle.run(checked)
    if (result !== null) {
      pub.reload()
    }
  }

  if (pub.status === "loading") return <SectionCard title="Publish in open format (Iceberg)"><LoadingSkeleton rows={2} /></SectionCard>
  if (pub.status === "error") return <SectionCard title="Publish in open format (Iceberg)"><ErrorState error={pub.error} onRetry={pub.reload} /></SectionCard>

  const p = pub.data
  const enabled = p.enabled
  const canEdit = p.canEdit
  const line = freshnessLine(p)
  const hasHistory = p.lastExportedAt !== null
  // PR slice D review D-B3: always show the freshness line and,
  // when a publish exists, the last published time and snapshot,
  // whether the switch is on or off.
  const showHistory = hasHistory && lastExport.status === "success" && lastExport.data

  return (
    <SectionCard
      title="Publish in open format (Iceberg)"
      description="A copy in open Iceberg format that outside tools can read. Publishing adds one full copy each time; old copies stay."
      action={
        <label className="flex items-center gap-2" data-testid="open-format-switch">
          <Switch
            checked={enabled}
            disabled={!canEdit || toggle.status === "pending"}
            aria-label="Publish in open format (Iceberg)"
            onCheckedChange={(checked) => {
              void handleToggle(checked)
            }}
            data-testid="open-format-toggle"
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
        {/* PR slice D review D-S2: render toggle error from status,
            not from the stale closure capture of toggle.error. */}
        {toggle.status === "error" && (
          <p role="alert" className="text-sm text-destructive">{toggle.error.message}</p>
        )}
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <span>Status:</span>
          <span className={stateLabelClass(line)}>
            {line}
          </span>
        </div>
        {!enabled && hasHistory && (
          <p className="text-xs text-muted-foreground">
            Publishing is off; this copy is no longer updated.
          </p>
        )}
        {showHistory && (
          <dl className="grid grid-cols-3 gap-1 text-sm">
            <dt className="text-muted-foreground">Last published</dt>
            <dd className="col-span-2">{lastExport.data!.exportedAt ? formatDateTime(lastExport.data!.exportedAt) : "—"}</dd>
            <dt className="text-muted-foreground">Snapshot</dt>
            <dd className="col-span-2 font-mono text-xs">
              {fmtMeasured(lastExport.data!.snapshotId, (v) => String(v))}
            </dd>
          </dl>
        )}
        {enabled && (
          <>
            <div className="flex flex-col items-start gap-1">
              {/* PR slice D review D-S3: "Publish now" when canEdit is
                  false is confusing — a user who cannot toggle also
                  cannot export. Show it only when they can. */}
              {canEdit && (
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
              )}
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