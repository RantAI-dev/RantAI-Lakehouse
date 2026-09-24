"use client"

import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { connectorService } from "@/services"

/**
 * The connector's recorded connectivity probes, newest first, from
 * `GET /api/connectors/{id}/probe-history`. Only rows the API returns are
 * shown: an empty history says the connector has not been tested, which is
 * a different claim from "no problems found". A probe that never measured a
 * latency shows an em dash, never a 0.
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
    <div>
      <p className="text-xs font-medium text-muted-foreground">Test history</p>
      {results.length === 0 ? (
        <p className="mt-1 text-sm text-muted-foreground" data-testid="probe-history-empty">
          Not tested yet. Only tests this build can run are recorded.
        </p>
      ) : (
        <ul className="mt-1 space-y-1 text-sm">
          {results.map((r, i) => (
            // The contract has no row id; list position is stable for one
            // fetch, and `testedAt` alone can repeat.
            <li key={i} className={r.ok ? undefined : "text-destructive"}>
              {r.ok ? "Passed" : "Failed"} · {r.message}
              <span className="ml-2 text-xs text-muted-foreground">
                {r.latencyMs === null ? "—" : `${r.latencyMs} ms`} ·{" "}
                {formatRelativeTime(r.testedAt)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
