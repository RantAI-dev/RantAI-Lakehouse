"use client"

import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatDateTime, formatRelativeTime } from "@/lib/format"
import { connectorService } from "@/services"

/**
 * The connector's recorded connectivity probes, newest first, from
 * `GET /api/connectors/{id}/probe-history`. Only rows the API returns are
 * shown: an empty history says the connector has not been tested, which is
 * a different claim from "no problems found". A probe that never measured a
 * latency shows an em dash, never a 0. One row per probe: a Passed or Failed
 * pill (the tones the run list uses), the message in the normal text colour
 * whatever the verdict, and at the right the latency and when, with the full
 * date and time on hover.
 *
 * `refreshKey` lets the parent re-fetch after it runs a new test, since a
 * successful `POST .../test` is exactly what appends a row here.
 */
export function ConnectorProbeHistoryPanel({
  connectorId,
  refreshKey = 0,
}: {
  connectorId: string
  refreshKey?: number
}) {
  const history = useService(
    (s) => connectorService.listProbeHistory(connectorId, undefined, s),
    [connectorId, refreshKey]
  )

  if (history.status === "loading") return <LoadingSkeleton rows={3} />
  if (history.status === "error")
    return <ErrorState error={history.error} onRetry={history.reload} />

  const results = history.data.results
  return (
    <SectionCard size="sm" title="Test history">
      {results.length === 0 ? (
        <p className="text-sm text-muted-foreground" data-testid="probe-history-empty">
          Not tested yet. Only tests this build can run are recorded.
        </p>
      ) : (
        <ul className="divide-y divide-border text-sm">
          {results.map((r, i) => (
            // The contract has no row id; list position is stable for one
            // fetch, and `testedAt` alone can repeat.
            <li key={i} className="flex flex-wrap items-center gap-x-3 gap-y-0.5 py-2 first:pt-0 last:pb-0">
              <Pill tone={r.ok ? "success" : "destructive"}>{r.ok ? "Passed" : "Failed"}</Pill>
              <span className="min-w-0 flex-1 basis-48 text-sm break-words">{r.message}</span>
              <span className="shrink-0 text-xs text-muted-foreground tabular-nums" title={formatDateTime(r.testedAt)}>
                {r.latencyMs === null ? "—" : `${r.latencyMs} ms`} · {formatRelativeTime(r.testedAt)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </SectionCard>
  )
}
