"use client"

import * as React from "react"
import Link from "next/link"
import { TriangleAlertIcon } from "lucide-react"
import { MetadataList } from "@/components/patterns/metadata-list"
import { Pill } from "@/components/patterns/status-badge"
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
import { DIRECTION_LABEL } from "./connectors-columns"

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
 * The drawer's first tab: what the connector is, where it connects, what it
 * ingests and when, and what needs a look. Every value comes from the API;
 * a part that could not be loaded says so instead of showing an empty or
 * zero value that would read as a real one.
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

  return (
    <div className="flex flex-col gap-5">
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

      <OverviewSection title="Details">
        <MetadataList
          items={[
            { label: "Type", value: detail.type },
            { label: "Direction", value: DIRECTION_LABEL[detail.direction] },
            { label: "Environment", value: detail.environment || "—" },
            { label: "Tenant", value: detail.tenant || "Unassigned" },
            { label: "Residency", value: detail.residency || "—" },
            { label: "Owner", value: detail.owner || "—" },
            {
              label: "Credential",
              value: detail.credentialManaged ? "Stored by lakehouse" : "Provisioned on the server",
            },
            {
              label: "Last test",
              value: detail.lastTestAt === null ? "Never tested" : formatRelativeTime(detail.lastTestAt),
            },
          ]}
        />
      </OverviewSection>

      <OverviewSection
        title="Connection"
        action={
          <Link href={`/connectors/${id}/edit`} className="text-xs font-medium text-primary hover:underline">
            Edit
          </Link>
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
      </OverviewSection>

      <OverviewSection
        title="Ingest"
        action={
          <button
            type="button"
            onClick={() => onOpenTab("ingest")}
            className="text-xs font-medium text-primary hover:underline"
          >
            Manage
          </button>
        }
      >
        {spec.status === "loading" ? (
          <Skeleton className="h-12 w-full" />
        ) : spec.status === "error" || !adapter ? (
          <p className="text-sm text-muted-foreground">Nothing to ingest until the connection is saved.</p>
        ) : (
          <MetadataList
            items={[
              {
                label: adapter === "cdc" ? "Tables streamed" : "Tables",
                value:
                  spec.data.sourceObjects.length === 0
                    ? "None picked yet"
                    : `${spec.data.sourceObjects.length} ${spec.data.sourceObjects.length === 1 ? "table" : "tables"}`,
              },
              {
                label: "Schedule",
                value:
                  adapter === "cdc" ? (
                    "Streams continuously"
                  ) : (
                    <>
                      {scheduleLabel(spec.data.scheduleCron)}
                      {spec.data.nextRunAt ? (
                        <span className="block text-xs text-muted-foreground">
                          Next {formatDateTime(spec.data.nextRunAt)} ({formatTimeUntil(spec.data.nextRunAt)})
                        </span>
                      ) : null}
                    </>
                  ),
              },
              {
                label: "Last run",
                value:
                  runs.status === "loading" ? (
                    "…"
                  ) : runs.status === "error" ? (
                    <span className="text-xs text-muted-foreground">Could not ask the orchestrator</span>
                  ) : lastRun ? (
                    <span className="flex flex-wrap items-center gap-1.5">
                      <Pill tone={RUN_STATUS_TONE[lastRun.status]}>{RUN_STATUS_LABEL[lastRun.status]}</Pill>
                      <span className="text-xs text-muted-foreground">{formatRelativeTime(lastRun.startedAt)}</span>
                    </span>
                  ) : (
                    "Never run"
                  ),
              },
            ]}
          />
        )}
      </OverviewSection>

      <OverviewSection title="Used by">
        {detail.dependentPipelines.length === 0 ? (
          <p className="text-sm text-muted-foreground">No pipeline reads from this connector.</p>
        ) : (
          <>
            <ul className="space-y-1">
              {detail.dependentPipelines.map((p) => (
                <li key={p.id} className="text-sm">
                  <Link href={`/pipelines/${p.id}`} className="font-mono text-primary hover:underline">
                    {p.name}
                  </Link>
                </li>
              ))}
            </ul>
            <p className="mt-1.5 text-xs text-muted-foreground">
              The connector cannot be deleted while these read from it.
            </p>
          </>
        )}
      </OverviewSection>
    </div>
  )
}

function OverviewSection({
  title,
  action,
  children,
}: {
  title: string
  action?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <section className="space-y-2">
      <div className="flex items-center justify-between gap-2 border-b border-border pb-1.5">
        <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{title}</h3>
        {action}
      </div>
      {children}
    </section>
  )
}
