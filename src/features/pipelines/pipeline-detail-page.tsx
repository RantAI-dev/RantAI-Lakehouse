"use client"

import * as React from "react"
import Link from "next/link"
import { useParams } from "next/navigation"
import { PauseIcon, PlayIcon, RotateCcwIcon, SquareIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { FlowCanvas } from "@/components/patterns/flow-canvas"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { MetadataList } from "@/components/patterns/metadata-list"
import { EntityHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import {
  formatCompactNumber,
  formatDateTime,
  formatRelativeTime,
  isPast,
} from "@/lib/format"
import { pipelineService } from "@/services"
import type { PipelineRun } from "@/services/contracts/pipelines"
import { getPipelineRunColumns, runDuration } from "./pipeline-run-columns"

function AssetLink({ id, label }: { readonly id?: string; readonly label: string }) {
  if (!id) return <span className="font-mono text-xs">{label}</span>
  return (
    <Link
      href={`/data/assets/${id}`}
      className="font-mono text-xs text-primary hover:underline"
    >
      {label}
    </Link>
  )
}

function RunDrawerActions({
  run,
  onCancel,
  onRetry,
  retrying,
}: {
  readonly run: PipelineRun
  readonly onCancel: () => void
  readonly onRetry: () => void
  readonly retrying: boolean
}) {
  const isRunning = run.status === "running"
  const canRetry = run.status === "failed" || run.status === "cancelled"

  return (
    <div className="flex flex-wrap gap-2">
      {isRunning ? (
        <Button size="sm" variant="outline" onClick={onCancel}>
          <SquareIcon data-icon="inline-start" />
          Cancel run
        </Button>
      ) : null}
      {canRetry ? (
        <Button size="sm" disabled={retrying} onClick={onRetry}>
          <RotateCcwIcon data-icon="inline-start" />
          {retrying ? "Retrying…" : "Retry run"}
        </Button>
      ) : null}
      {run.outputAssetId ? (
        <Button
          size="sm"
          variant="ghost"
          render={<Link href={`/data/assets/${run.outputAssetId}`} />}
        >
          Output dataset
        </Button>
      ) : null}
      <Button
        size="sm"
        variant="ghost"
        render={<Link href={`/lineage?focus=${run.pipelineId}`} />}
      >
        Lineage
      </Button>
      {run.auditEventId ? (
        <Button
          size="sm"
          variant="ghost"
          render={<Link href={`/audit?event=${run.auditEventId}`} />}
        >
          Audit
        </Button>
      ) : null}
    </div>
  )
}

function RunDrawerContent({
  run,
  onCancel,
  onRetry,
  retrying,
}: {
  readonly run: PipelineRun
  readonly onCancel: () => void
  readonly onRetry: () => void
  readonly retrying: boolean
}) {
  const metadataItems = [
    { label: "Status", value: <StatusBadge status={run.status} /> },
    { label: "Started", value: formatDateTime(run.startedAt) },
    {
      label: "Ended",
      value: run.endedAt ? formatDateTime(run.endedAt) : "running",
    },
    { label: "Duration", value: runDuration(run) },
    // These read "—" unless the orchestrator reported them. They used to
    // be zeros, which looked like a pipeline that had processed nothing.
    { label: "Processed", value: formatCompactNumber(run.processed) },
    { label: "Accepted", value: formatCompactNumber(run.accepted) },
    { label: "Rejected", value: formatCompactNumber(run.rejected) },
    { label: "Retried", value: formatCompactNumber(run.retried) },
    {
      label: "Checkpoint",
      value: run.checkpoint ? (
        <span className="font-mono text-xs">{run.checkpoint}</span>
      ) : (
        "—"
      ),
    },
    {
      label: "Pipeline",
      value: <span className="font-mono text-xs">{run.pipelineId}</span>,
    },
  ]

  return (
    <>
      <RunDrawerActions
        run={run}
        onCancel={onCancel}
        onRetry={onRetry}
        retrying={retrying}
      />
      <MetadataList items={metadataItems} />
      {run.error ? (
        <div>
          <p className="text-xs font-medium text-muted-foreground">Error</p>
          <p className="mt-1 text-sm text-destructive">{run.error}</p>
        </div>
      ) : null}
    </>
  )
}

function RunDrawer({
  run,
  onClose,
  onChanged,
}: {
  readonly run: PipelineRun | null
  readonly onClose: () => void
  readonly onChanged: () => void
}) {
  const cancelAction = useServiceAction(
    withNotify(
      { success: "Run cancelled", error: "Failed to cancel run" },
      (signal, runId: string) => pipelineService.cancelRun(runId, signal)
    )
  )
  const retryAction = useServiceAction(
    withNotify(
      { success: "Run retried", error: "Failed to retry run" },
      (signal, runId: string) => pipelineService.retryRun(runId, signal)
    )
  )
  const [cancelOpen, setCancelOpen] = React.useState(false)

  const handleRetry = async () => {
    if (!run) return
    const next = await retryAction.run(run.id)
    if (next) {
      onChanged()
      onClose()
    }
  }

  const handleConfirmCancel = async () => {
    if (!run) return
    const updated = await cancelAction.run(run.id)
    if (updated) {
      setCancelOpen(false)
      onChanged()
      onClose()
    }
  }

  return (
    <>
      <DetailDrawer
        open={run !== null}
        onOpenChange={(open) => {
          if (!open) onClose()
        }}
        title="Run details"
        description={run ? `Run ${run.id}` : undefined}
      >
        {run ? (
          <RunDrawerContent
            run={run}
            onCancel={() => setCancelOpen(true)}
            onRetry={handleRetry}
            retrying={retryAction.status === "pending"}
          />
        ) : null}
      </DetailDrawer>
      <ConfirmActionDialog
        open={cancelOpen}
        onOpenChange={setCancelOpen}
        title="Cancel pipeline run"
        description={run ? `Cancel run ${run.id}?` : "Cancel this run?"}
        impact="In-flight work stops at the last checkpoint. Partial output may remain."
        confirmLabel="Cancel run"
        confirming={cancelAction.status === "pending"}
        onConfirm={handleConfirmCancel}
      />
    </>
  )
}

export function PipelineDetailPage() {
  const { pipelineId } = useParams<{ pipelineId: string }>()
  const state = useService(
    (s) => pipelineService.getPipeline(pipelineId, s),
    [pipelineId]
  )
  const [selectedRun, setSelectedRun] = React.useState<PipelineRun | null>(null)
  const [cancelRunTarget, setCancelRunTarget] = React.useState<PipelineRun | null>(null)
  const [pauseOpen, setPauseOpen] = React.useState(false)

  const runAction = useServiceAction(
    withNotify(
      { success: "Run triggered", error: "Failed to trigger run" },
      (signal, id: string) => pipelineService.triggerRun(id, signal)
    )
  )
  const pauseAction = useServiceAction(
    withNotify(
      { success: "Pipeline paused", error: "Failed to pause pipeline" },
      (signal, id: string) => pipelineService.pausePipeline(id, signal)
    )
  )
  const resumeAction = useServiceAction(
    withNotify(
      { success: "Pipeline resumed", error: "Failed to resume pipeline" },
      (signal, id: string) => pipelineService.resumePipeline(id, signal)
    )
  )
  const cancelAction = useServiceAction(
    withNotify(
      { success: "Run cancelled", error: "Failed to cancel run" },
      (signal, runId: string) => pipelineService.cancelRun(runId, signal)
    )
  )
  const retryAction = useServiceAction(
    withNotify(
      { success: "Run retried", error: "Failed to retry run" },
      (signal, runId: string) => pipelineService.retryRun(runId, signal)
    )
  )

  const runs = React.useMemo(() => state.data?.runs ?? [], [state.data?.runs])

  const columns = React.useMemo(
    () =>
      getPipelineRunColumns({
        onSelect: setSelectedRun,
        onCancel: (run) => setCancelRunTarget(run),
        onRetry: async (run) => {
          const next = await retryAction.run(run.id)
          if (next) state.reload()
        },
      }),
    [retryAction, state]
  )

  const { table } = useDataTable({
    data: runs,
    columns,
    pageCount: 1,
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  if (state.status === "loading") return <LoadingSkeleton rows={8} />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  const p = state.data
  const isPaused = p.status === "paused"
  // Pause/Resume act on a schedule in the orchestrator; an authored
  // pipeline has neither.
  const canRun = p.origin === "orchestrator"

  return (
    <div className="flex flex-col gap-4">
      <EntityHeader
        eyebrow={<Link href="/pipelines" className="hover:underline">Pipelines</Link>}
        title={p.name}
        titleAccessory={<StatusBadge status={p.status} />}
        description={p.description ?? undefined}
        actions={
          // An authored pipeline has no job in the orchestrator, so Run /
          // Pause / Resume have nothing to act on. They used to be shown
          // anyway and answered 503, which reads as "try again later".
          !canRun ? (
            <span className="text-xs text-muted-foreground">
              No engine attached — this pipeline cannot run yet.
            </span>
          ) : (
          <>
            {isPaused ? (
              <Button
                variant="outline"
                size="sm"
                disabled={resumeAction.status === "pending"}
                onClick={async () => {
                  const updated = await resumeAction.run(pipelineId)
                  if (updated) state.reload()
                }}
              >
                <PlayIcon data-icon="inline-start" />
                {resumeAction.status === "pending" ? "Resuming…" : "Resume"}
              </Button>
            ) : (
              <Button
                variant="outline"
                size="sm"
                disabled={pauseAction.status === "pending"}
                onClick={() => setPauseOpen(true)}
              >
                <PauseIcon data-icon="inline-start" />
                Pause
              </Button>
            )}
            <Button
              size="sm"
              disabled={isPaused || runAction.status === "pending"}
              onClick={async () => {
                const run = await runAction.run(pipelineId)
                if (run) state.reload()
              }}
            >
              <PlayIcon data-icon="inline-start" />
              {runAction.status === "pending" ? "Starting…" : "Run now"}
            </Button>
          </>
          )
        }
      />
      <ConfirmActionDialog
        open={pauseOpen}
        onOpenChange={setPauseOpen}
        title="Pause pipeline"
        description={`Pause ${p.name}? Scheduled runs will stop until resumed.`}
        impact="In-flight runs continue; new triggers are held."
        confirmLabel="Pause pipeline"
        confirming={pauseAction.status === "pending"}
        onConfirm={async () => {
          const updated = await pauseAction.run(pipelineId)
          if (updated) {
            setPauseOpen(false)
            state.reload()
          }
        }}
      />
      <ConfirmActionDialog
        open={cancelRunTarget !== null}
        onOpenChange={(open) => {
          if (!open) setCancelRunTarget(null)
        }}
        title="Cancel pipeline run"
        description={cancelRunTarget ? `Cancel run ${cancelRunTarget.id}?` : "Cancel this run?"}
        impact="In-flight work stops at the last checkpoint. Partial output may remain."
        confirmLabel="Cancel run"
        confirming={cancelAction.status === "pending"}
        onConfirm={async () => {
          if (!cancelRunTarget) return
          const updated = await cancelAction.run(cancelRunTarget.id)
          if (updated) {
            setCancelRunTarget(null)
            state.reload()
          }
        }}
      />
      <Tabs defaultValue="overview">
        <TabsList>
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="graph">Graph</TabsTrigger>
          <TabsTrigger value="runs">Runs</TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="mt-3">
          <SectionCard title="Configuration">
            <MetadataList
              columns={3}
              items={[
                { label: "Owner", value: p.owner },
                { label: "Kind", value: p.kind },
                { label: "Schedule", value: <span className="font-mono text-xs">{p.schedule}</span> },
                {
                  label: "Source",
                  value: <AssetLink id={p.sourceAssetId} label={p.source} />,
                },
                {
                  label: "Target",
                  value: <AssetLink id={p.targetAssetId} label={p.target} />,
                },
                {
                  label: "Connector",
                  value: p.connectorId ? (
                    <Link
                      href={`/connectors`}
                      className="font-mono text-xs text-primary hover:underline"
                    >
                      {p.connectorId}
                    </Link>
                  ) : (
                    "—"
                  ),
                },
                {
                  label: "Last run",
                  value: p.lastRunAt ? formatRelativeTime(p.lastRunAt) : "Never",
                },
                {
                  label: "Next run",
                  // A scheduled time that has already passed is not a
                  // future event: "14d ago" under "Next run" reads as a
                  // rendering bug when it is really an overdue schedule.
                  value: p.nextRunAt ? (
                    isPast(p.nextRunAt) ? (
                      <span className="text-amber-600 dark:text-amber-400">
                        Overdue · due {formatRelativeTime(p.nextRunAt)}
                      </span>
                    ) : (
                      formatRelativeTime(p.nextRunAt)
                    )
                  ) : (
                    "—"
                  ),
                },
                { label: "SLA", value: p.slaOk ? "OK" : "Breached" },
                { label: "Freshness", value: <FreshnessIndicator lagSeconds={p.freshnessLagSeconds} /> },
                ...p.configSummary.map((c) => ({
                  label: c.key,
                  value: <span className="font-mono text-xs">{c.value}</span>,
                })),
              ]}
            />
          </SectionCard>
        </TabsContent>
        <TabsContent value="graph" className="mt-3">
          {p.graph.length === 0 ? (
            <EmptyState
              title="No graph for this pipeline"
              description="The orchestrator's job list does not describe the steps, so there is nothing here to draw."
            />
          ) : (
            <FlowCanvas
              nodes={p.graph.map((n) => ({
                id: n.id,
                label: n.label,
                kind: n.kind,
                status: n.status,
              }))}
            />
          )}
        </TabsContent>
        <TabsContent value="runs" className="mt-3">
          {p.runs.length === 0 ? (
            <EmptyState
              title="No runs"
              description={
                p.runsUnavailable ??
                "Runs appear here once the pipeline executes."
              }
            />
          ) : (
            <DataTable
              table={table}
              onRowClick={setSelectedRun}
              infinite={{
                onLoadMore: () => {},
                hasNextPage: false,
                isFetchingNextPage: false,
                totalItems: runs.length,
                loadedCount: runs.length,
              }}
            />
          )}
        </TabsContent>
      </Tabs>
      <RunDrawer
        run={selectedRun}
        onClose={() => setSelectedRun(null)}
        onChanged={state.reload}
      />
    </div>
  )
}
