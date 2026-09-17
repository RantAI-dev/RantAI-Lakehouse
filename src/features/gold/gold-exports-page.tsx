"use client"

import { RefreshCwIcon } from "lucide-react"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
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
import { formatCompactNumber, formatDateTime } from "@/lib/format"
import { fmtMeasured } from "@/lib/measured"
import { goldService } from "@/services"
import type { GoldExportRun, GoldMart, GoldReadBack } from "@/services/contracts/gold"

/**
 * One mart's row: last export (read straight back from Iceberg), export
 * history count, best-effort consumer count, and an "Export now" trigger.
 * Each sub-fetch is independent — `getLastExport` 404/500s for a mart that
 * has never been exported (Task 2's `read_back` has no table to read yet),
 * and that is rendered as "Never exported" here, not as a page-level
 * error.
 */
function MartRow({ mart }: { mart: GoldMart }) {
  const lastExport = useService<GoldReadBack | null>(async (signal) => {
    try {
      return await goldService.getLastExport(mart.name, signal)
    } catch {
      return null
    }
  }, [mart.name])
  const runs = useService<GoldExportRun[]>(
    (signal) => goldService.listExportRuns(mart.name, signal),
    [mart.name]
  )
  const consumers = useService(
    (signal) => goldService.getConsumers(mart.name, signal),
    [mart.name]
  )
  const exportNow = useServiceAction((signal) => goldService.triggerExport(mart.name, signal))

  return (
    <TableRow>
      <TableCell className="font-medium">{mart.name}</TableCell>
      <TableCell>{formatCompactNumber(mart.rows)}</TableCell>
      <TableCell>
        {lastExport.status === "loading" ? (
          <span className="text-muted-foreground">…</span>
        ) : lastExport.data?.exportedAt ? (
          formatDateTime(lastExport.data.exportedAt)
        ) : (
          <span className="text-muted-foreground">Never exported</span>
        )}
      </TableCell>
      <TableCell className="font-mono text-xs">
        {lastExport.status === "success" && lastExport.data
          ? fmtMeasured(lastExport.data.snapshotId)
          : "—"}
      </TableCell>
      <TableCell>
        {runs.status === "loading" ? (
          <span className="text-muted-foreground">…</span>
        ) : runs.status === "error" ? (
          <span className="text-destructive">—</span>
        ) : (
          runs.data.length
        )}
      </TableCell>
      <TableCell>
        {consumers.status === "loading" ? (
          <span className="text-muted-foreground">…</span>
        ) : consumers.status === "error" ? (
          <span className="text-destructive">—</span>
        ) : consumers.data.supported ? (
          String(consumers.data.consumers?.length ?? 0)
        ) : (
          <span
            className="text-muted-foreground"
            title={consumers.data.reason ?? "Not measured"}
          >
            Not measured
          </span>
        )}
      </TableCell>
      <TableCell>
        <div className="flex flex-col items-end gap-1">
          <Button
            size="sm"
            variant="outline"
            disabled={exportNow.status === "pending"}
            onClick={() => {
              void exportNow.run().then((result) => {
                if (result) {
                  lastExport.reload()
                  runs.reload()
                }
              })
            }}
          >
            <RefreshCwIcon data-icon="inline-start" />
            {exportNow.status === "pending" ? "Exporting…" : "Export now"}
          </Button>
          {exportNow.status === "error" ? (
            <p role="alert" className="text-xs text-destructive">
              {exportNow.error.message}
            </p>
          ) : null}
        </div>
      </TableCell>
    </TableRow>
  )
}

/**
 * Gold Exports console page (WS6 item 10) — every `serving.*` mart
 * (`GET /api/dashboard/fields`), each with its last export (read back
 * from Iceberg, independent of what the export claimed), export history
 * count, best-effort consumer count, and a manual "Export now" action.
 */
export function GoldExportsPage() {
  const marts = useService<GoldMart[]>((signal) => goldService.listMarts(signal), [])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Gold Exports"
        description="Serving marts exported to the Gold Iceberg layer (ADR 0010). Each row's snapshot and export time come from reading Iceberg back, not from what the last export claimed."
      />
      {marts.status === "loading" ? <LoadingSkeleton /> : null}
      {marts.status === "error" ? (
        <ErrorState error={marts.error} onRetry={marts.reload} />
      ) : null}
      {marts.status === "success" ? (
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead>Mart</TableHead>
              <TableHead>Rows</TableHead>
              <TableHead>Last export</TableHead>
              <TableHead>Snapshot</TableHead>
              <TableHead>History (runs)</TableHead>
              <TableHead>Consumers (7d)</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            {marts.data.map((mart) => (
              <MartRow key={mart.name} mart={mart} />
            ))}
          </TableBody>
        </Table>
      ) : null}
    </div>
  )
}
