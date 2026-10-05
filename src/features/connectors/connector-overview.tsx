"use client"

import Link from "next/link"
import { TriangleAlertIcon } from "lucide-react"
import { HealthTile } from "@/components/patterns/health-tile"
import { MetadataList } from "@/components/patterns/metadata-list"
import { SectionCard } from "@/components/patterns/section-card"
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useService } from "@/hooks/use-service"
import { formatDateTime, formatRelativeTime, formatTimeUntil } from "@/lib/format"
import { connectorService } from "@/services"
import type {
  ConnectorDetail,
  ConnectorProbeResult,
  IngestJobRun,
  IngestRun,
} from "@/services/contracts/connectors"
import {
  RUN_STATUS_LABEL,
  RUN_STATUS_TONE,
  groupResultsByRun,
  isActive,
  scheduleLabel,
} from "./connector-ingest-panel"
import { connectionReviewItems } from "./connector-review"

/** Something about the connector that needs a look, newest source first. */
export type Problem = { key: string; message: string; at: string | null }

/** How many failed tables of one run are named before "and N more". */
const TABLES_NAMED = 3

/**
 * What currently needs attention, from real results only: the latest
 * connection test if it failed (an older failure a later pass resolved is
 * history, not a problem), and the latest finished ingest run if it failed
 * or left tables unloaded. `null` inputs (not loaded, or could not be
 * loaded) contribute nothing rather than a guess.
 */
export function recentProblems(
  probes: ConnectorProbeResult[] | null,
  runs: IngestJobRun[] | null,
  results: IngestRun[] | null
): Problem[] {
  const problems: Problem[] = []
  const latestProbe = probes?.[0]
  if (latestProbe && !latestProbe.ok) {
    problems.push({
      key: "probe",
      message: `Connection test failed: ${latestProbe.message}`,
      at: latestProbe.testedAt,
    })
  }
  if (runs) {
    const finished = groupResultsByRun(runs, results ?? []).find((g) => g.run !== null && !isActive(g.run))
    const run = finished?.run
    if (finished && run) {
      const failedTables = finished.results.filter((r) => r.status !== "succeeded")
      if (run.status === "failed" && finished.results.length === 0) {
        problems.push({
          key: "run",
          message: "The last ingest run failed before it reached any table.",
          at: run.startedAt,
        })
      } else if (run.status === "failed" || failedTables.length > 0) {
        const named = failedTables
          .slice(0, TABLES_NAMED)
          .map((r) => `${r.object} (${r.error || r.status})`)
          .join(", ")
        const more = failedTables.length > TABLES_NAMED ? ` and ${failedTables.length - TABLES_NAMED} more` : ""
        problems.push({
          key: "run",
          message:
            failedTables.length > 0
              ? `The last ingest run did not load ${failedTables.length} ${
                  failedTables.length === 1 ? "table" : "tables"
                }: ${named}${more}.`
              : "The last ingest run failed.",
          at: run.startedAt,
        })
      }
    }
  }
  return problems
}

/**
 * The connector page's first tab: a strip of four tiles (health, tables,
 * schedule, last run, each opening the tab that holds the detail behind it),
 * then where the connector connects and who reads from it. What the connector
 * is (type, tenant, owner, ...) is the page header's row of facts, not
 * repeated here. Every value comes from the API; a part that could not be
 * loaded says so instead of showing an empty or zero value that would read as
 * a real one. `onOpenTab` changes the page's open tab. "Needs attention",
 * when there is something, stays above the tiles.
 */
export function ConnectorOverview({
  detail,
  onOpenTab,
}: {
  detail: ConnectorDetail
  onOpenTab: (tab: "ingest" | "tests") => void
}) {
  const id = detail.id
  const spec = useService((s) => connectorService.getIngestSpec(id, s), [id])
  const probes = useService((s) => connectorService.listProbeHistory(id, 1, s), [id])
  const runs = useService((s) => connectorService.listIngestJobRuns(id, s), [id])
  const results = useService((s) => connectorService.listIngestRuns(id, s), [id])

  const problems = recentProblems(probes.data?.results ?? null, runs.data, results.data)
  const lastRun = runs.data?.[0] ?? null
  const adapter = spec.data?.adapter ?? null
  const tables = spec.data?.sourceObjects.length ?? 0

  // What the two ingest tiles say when the spec is not there to read: still
  // loading, could not be loaded, or no connection saved yet (so nothing can
  // be ingested). Never a zero or a blank.
  const specGap =
    spec.status === "loading" ? (
      <Skeleton className="h-5 w-20" />
    ) : spec.status === "error" ? (
      <span className="text-muted-foreground">Could not be loaded</span>
    ) : !adapter ? (
      <span className="text-muted-foreground">Not set up yet</span>
    ) : null
  // The error's own sentence is on the Connection card below; repeating it in
  // two tiles would only be noise.
  const specGapHint = spec.status === "success" && !adapter ? "Save the connection first" : undefined

  return (
    <div className="flex flex-col gap-2">
      {problems.length > 0 ? (
        <section
          aria-labelledby="connector-problems"
          className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2.5"
        >
          <h3 id="connector-problems" className="flex items-center gap-1.5 text-sm font-medium text-destructive">
            <TriangleAlertIcon className="size-4" />
            Needs attention
          </h3>
          <ul className="mt-1.5 space-y-1">
            {problems.map((p) => (
              <li key={p.key} className="text-xs text-foreground">
                {p.message}
                {p.at ? <span className="ml-1.5 text-muted-foreground">{formatRelativeTime(p.at)}</span> : null}{" "}
                <button
                  type="button"
                  onClick={() => onOpenTab(p.key === "probe" ? "tests" : "ingest")}
                  className="font-medium text-primary hover:underline"
                >
                  {p.key === "probe" ? "Test history" : "Runs"}
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
        <HealthTile
          label="Health"
          hint={detail.lastTestAt === null ? "Never tested" : `Tested ${formatRelativeTime(detail.lastTestAt)}`}
          onClick={() => onOpenTab("tests")}
        >
          <HealthBadge health={detail.health} />
        </HealthTile>
        <HealthTile
          label={adapter === "cdc" ? "Tables streamed" : "Tables"}
          hint={specGapHint}
          onClick={() => onOpenTab("ingest")}
        >
          {specGap ??
            (tables === 0 ? (
              <span className="text-muted-foreground">None picked yet</span>
            ) : (
              <span className="tabular-nums">
                {tables} {tables === 1 ? "table" : "tables"}
              </span>
            ))}
        </HealthTile>
        <HealthTile
          label="Schedule"
          hint={
            spec.status === "success" && adapter && adapter !== "cdc" && spec.data.nextRunAt
              ? `Next ${formatDateTime(spec.data.nextRunAt)} (${formatTimeUntil(spec.data.nextRunAt)})`
              : specGapHint
          }
          onClick={() => onOpenTab("ingest")}
        >
          {specGap ?? (adapter === "cdc" ? "Streams continuously" : scheduleLabel(spec.data?.scheduleCron ?? null))}
        </HealthTile>
        <HealthTile
          label="Last run"
          hint={
            lastRun ? (
              <span title={lastRun.startedAt ? formatDateTime(lastRun.startedAt) : undefined}>
                {formatRelativeTime(lastRun.startedAt)}
              </span>
            ) : undefined
          }
          onClick={() => onOpenTab("ingest")}
        >
          {runs.status === "loading" ? (
            <Skeleton className="h-5 w-20" />
          ) : runs.status === "error" ? (
            <span className="text-muted-foreground">Could not ask the orchestrator</span>
          ) : lastRun ? (
            <Pill tone={RUN_STATUS_TONE[lastRun.status]}>{RUN_STATUS_LABEL[lastRun.status]}</Pill>
          ) : (
            <span className="text-muted-foreground">Never run</span>
          )}
        </HealthTile>
      </div>

      <div className="grid items-start gap-2 xl:grid-cols-2">
        <SectionCard
          size="sm"
          title="Connection"
          action={
            <Button size="sm" variant="ghost" render={<Link href={`/connectors/${id}/edit`} />}>
              Edit
            </Button>
          }
        >
          {spec.status === "loading" ? (
            <Skeleton className="h-12 w-full" />
          ) : spec.status === "error" ? (
            <p className="text-xs text-destructive">Connection settings could not be loaded · {spec.error.message}</p>
          ) : !adapter ? (
            <p className="text-sm text-muted-foreground">No connection settings saved yet.</p>
          ) : (
            <MetadataList
              items={connectionReviewItems(adapter, spec.data.dial).map((item) => ({
                label: item.label,
                value: item.value || "—",
              }))}
            />
          )}
        </SectionCard>

        <SectionCard size="sm" title="Used by">
          {detail.dependentPipelines.length === 0 ? (
            <p className="text-sm text-muted-foreground">No pipeline reads from this connector.</p>
          ) : (
            <>
              <ul className="divide-y divide-border text-sm">
                {detail.dependentPipelines.map((p) => (
                  <li key={p.id} className="py-1.5 first:pt-0 last:pb-0">
                    <Link href={`/pipelines/${p.id}`} className="font-mono text-sm text-primary hover:underline">
                      {p.name}
                    </Link>
                  </li>
                ))}
              </ul>
              <p className="mt-2 text-xs text-muted-foreground">
                The connector cannot be deleted while these read from it.
              </p>
            </>
          )}
        </SectionCard>
      </div>
    </div>
  )
}
