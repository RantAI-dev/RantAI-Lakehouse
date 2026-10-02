"use client"

import * as React from "react"
import Link from "next/link"
import { useParams } from "next/navigation"
import { PauseIcon, PlayIcon, RotateCcwIcon, SquareIcon } from "lucide-react"
import { CodeView } from "@/components/patterns/code-view"
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
import { fmtMeasured } from "@/lib/measured"
import type { EntityStatus } from "@/lib/status"
import { pipelineService } from "@/services"
import type {
  PipelineOpNode,
  PipelineRun,
  PipelineRunLogsPage,
} from "@/services/contracts/pipelines"
import { topoSortOps } from "./topo-sort-ops"
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

/**
 * Source tab: an op picker plus its read-only text. A 404
 * (unknown op) or 409 (commit mismatch, `pipeline_source.rs`'s
 * `check_commit`) both surface through `source.status === "error"` with
 * the SERVER's own message — never a client-fabricated string that would
 * hide which of the two real reasons applies.
 *
 * Ops with no `sourceRef` are left out of the picker entirely, and a
 * pipeline where NO op declares one says so instead of offering a choice
 * that cannot resolve: `GET /api/pipelines/{id}/source` matches its `op=`
 * against the op's `sourceRef`, so an op whose code location publishes no
 * provenance metadata has nothing to look up.
 */
function SourceTab({ pipelineId, ops }: { pipelineId: string; ops: PipelineOpNode[] }) {
  const withSource = ops.filter((op) => op.sourceRef !== null)
  if (withSource.length === 0) {
    return (
      <EmptyState
        title="No source provenance"
        description="No op in this pipeline declares where its code lives, so there is nothing to show. A code location publishes this as op metadata (source_ref/commit)."
      />
    )
  }
  return <SourceViewer pipelineId={pipelineId} ops={withSource} />
}

/**
 * The picker itself, split out so [`SourceTab`]'s "nothing declares
 * provenance" branch can return before any hook runs — and so the
 * selected value is the op's `sourceRef` (what the route matches on),
 * while the label stays the op NAME the reader recognises from the graph.
 * Sending the name instead was why every op answered "source provenance
 * is unavailable for this build": no op's `sourceRef` equals its name, so
 * the lookup found no op at all and `check_commit` saw `None`.
 */
function SourceViewer({ pipelineId, ops }: { pipelineId: string; ops: PipelineOpNode[] }) {
  const [selectedRef, setSelectedRef] = React.useState(ops[0]?.sourceRef ?? "")
  const source = useService(
    (s) => pipelineService.getPipelineSource(pipelineId, selectedRef, s),
    [pipelineId, selectedRef]
  )
  return (
    <div className="flex flex-col gap-3">
      <select
        className="h-8 w-64 rounded-lg border border-input bg-transparent px-2.5 text-sm"
        value={selectedRef}
        onChange={(e) => setSelectedRef(e.target.value)}
      >
        {ops.map((op) => (
          <option key={op.name} value={op.sourceRef ?? ""}>
            {op.name}
          </option>
        ))}
      </select>
      {source.status === "loading" ? <LoadingSkeleton rows={6} /> : null}
      {source.status === "error" ? (
        <ErrorState error={source.error} onRetry={source.reload} />
      ) : null}
      {source.status === "success" ? (
        <div className="flex flex-col gap-2">
          <p className="text-xs text-muted-foreground">
            {source.data.sourceRef} · commit {source.data.commit.slice(0, 12)}
          </p>
          <CodeView text={source.data.text} />
        </div>
      ) : null}
    </div>
  )
}

/**
 * Steps + polled logs inside the run drawer. Steps are a
 * one-shot `useService` load (they only change when the drawer re-opens
 * on a new run). Logs poll by the backend's own opaque cursor — never
 * re-requested from zero — following the exact tick()/setTimeout shape
 * `src/features/copilot/build-tree.tsx`'s `BuildTree` already uses (the
 * only other polling precedent in this codebase): re-arm only while
 * `isLive`, clear the timer on unmount, never let a caught error stop
 * future attempts (it backs off to 5s instead).
 */
function RunStepsAndLogs({
  pipelineId,
  runId,
  isLive,
}: {
  pipelineId: string
  runId: string
  isLive: boolean
}) {
  const stepsState = useService((s) => pipelineService.getRunSteps(pipelineId, runId, s), [pipelineId, runId])
  const [logs, setLogs] = React.useState<PipelineRunLogsPage["lines"]>([])
  const [logsError, setLogsError] = React.useState<string | null>(null)

  React.useEffect(() => {
    let alive = true
    let cursor: string | undefined
    let timer: ReturnType<typeof setTimeout>

    async function tick() {
      try {
        const page = await pipelineService.getRunLogs(pipelineId, runId, cursor)
        if (!alive) return
        // Append-only: the backend's cursor is monotonic and opaque, so a
        // page never re-includes an already-returned line — appending
        // (never replacing) `logs` is what keeps polling from dropping or
        // duplicating lines.
        setLogs((prev) => [...prev, ...page.lines])
        cursor = page.cursor
        setLogsError(null)
        if (isLive) timer = setTimeout(tick, 3000)
      } catch (err) {
        if (alive) {
          setLogsError(err instanceof Error ? err.message : "failed to fetch logs")
          if (isLive) timer = setTimeout(tick, 5000)
        }
      }
    }
    setLogs([])
    tick()
    return () => {
      alive = false
      clearTimeout(timer)
    }
  }, [pipelineId, runId, isLive])

  return (
    <div className="flex flex-col gap-3">
      <div>
        <p className="mb-1 text-xs font-medium text-muted-foreground">Steps</p>
        {stepsState.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
        {stepsState.status === "error" ? (
          <ErrorState error={stepsState.error} onRetry={stepsState.reload} />
        ) : null}
        {stepsState.status === "success" && stepsState.data.length === 0 ? (
          <p className="text-xs text-muted-foreground">No steps recorded for this run yet.</p>
        ) : null}
        {stepsState.status === "success" && stepsState.data.length > 0 ? (
          <ul className="flex flex-col gap-1">
            {stepsState.data.map((step) => (
              <li key={step.stepKey} className="flex items-center justify-between text-xs">
                <span className="font-mono">{step.stepKey}</span>
                <span className="flex items-center gap-2">
                  <StatusBadge status={step.status} />
                  {fmtMeasured(
                    step.materializations.find((m) => m.rows !== null)?.rows ?? null,
                    formatCompactNumber
                  )}
                </span>
              </li>
            ))}
          </ul>
        ) : null}
      </div>
      <div>
        <p className="mb-1 text-xs font-medium text-muted-foreground">Logs</p>
        {logsError ? <p className="mb-1 text-xs text-destructive">{logsError}</p> : null}
        <pre className="max-h-48 overflow-y-auto rounded-md border border-border bg-muted/30 p-2 font-mono text-xs">
          {logs.length > 0
            ? logs.map((line) => `[${line.level}] ${line.message}`).join("\n")
            : "No log lines yet."}
        </pre>
      </div>
    </div>
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
          <>
            <RunDrawerContent
              run={run}
              onCancel={() => setCancelOpen(true)}
              onRetry={handleRetry}
              retrying={retryAction.status === "pending"}
            />
            <RunStepsAndLogs pipelineId={run.pipelineId} runId={run.id} isLive={run.status === "running"} />
          </>
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
  // Runs come from `GET /api/pipelines/{id}/runs`, NOT from the detail
  // payload: `routes::pipelines::detail` (WS4 item C1) returns id/name/
  // graph/config/definition and no `runs` field at all. Reading
  // `state.data.runs[0]` crashed every detail page with "Cannot read
  // properties of undefined (reading '0')" -- the contract declared a field
  // the route never sends, and a hand-written contract type cannot catch
  // that at compile time.
  const runsState = useService(
    (s) => pipelineService.listRuns(pipelineId, s),
    [pipelineId]
  )
  const runs = runsState.status === "success" ? runsState.data : []
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
  const activateAction = useServiceAction((signal, id: string) =>
    pipelineService.setPipelineStatus(id, "ready", signal)
  )

  // Latest run's real step statuses color the graph tab's nodes — `runs`
  // is returned most-recent-first (`list_runs_for_job`/`run_to_json`
  // ordering, unchanged by WS4).
  const latestRun = runs[0]
  const stepsState = useService(
    (s) => (latestRun ? pipelineService.getRunSteps(pipelineId, latestRun.id, s) : Promise.resolve([])),
    [pipelineId, latestRun?.id]
  )
  function latestRunStepStatus(opName: string): EntityStatus | undefined {
    if (stepsState.status !== "success") return undefined
    return stepsState.data.find((step) => step.stepKey === opName)?.status
  }

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
  const isDraft = p.status === "draft"
  // `Pipeline` carries no `origin` field — the API never sends one. An
  // authored (console-created) pipeline's id is always `pl-<slug>-<base36
  // millis>`; a Dagster job id is never prefixed `pl-` (same derivation
  // as `pipelineOrigin` in `./pipeline-columns.tsx`). Pause/Resume/Run act
  // on a schedule and engine in the orchestrator, which an authored
  // pipeline has neither.
  const canRun = !p.id.startsWith("pl-")

  return (
    <div className="flex flex-col gap-4">
      <EntityHeader
        eyebrow={<Link href="/pipelines" className="hover:underline">Pipelines</Link>}
        title={p.name}
        titleAccessory={<StatusBadge status={p.status} />}
        description={p.description ?? undefined}
        actions={
          isDraft ? (
            <Button
              size="sm"
              disabled={activateAction.status === "pending"}
              onClick={async () => {
                const updated = await activateAction.run(pipelineId)
                if (updated) state.reload()
              }}
            >
              <PlayIcon data-icon="inline-start" />
              {activateAction.status === "pending" ? "Activating…" : "Activate"}
            </Button>
          ) : !canRun ? (
            // An authored, non-draft pipeline still has no job in the
            // orchestrator, so Run/Pause/Resume have nothing to act on.
            // They used to be shown anyway and answered 503, which reads
            // as "try again later".
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
          <TabsTrigger value="source">Source</TabsTrigger>
          <TabsTrigger value="runs">Runs</TabsTrigger>
          <TabsTrigger value="config">Config</TabsTrigger>
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
                  value: <AssetLink id={p.sourceAssetId} label={p.source ?? "—"} />,
                },
                {
                  label: "Target",
                  value: <AssetLink id={p.targetAssetId} label={p.target ?? "—"} />,
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
                { label: "Last run", value: p.lastRunAt === null ? "—" : formatRelativeTime(p.lastRunAt) },
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
                { label: "SLA", value: p.slaOk === null ? "—" : p.slaOk ? "OK" : "Breached" },
                { label: "Freshness", value: <FreshnessIndicator lagSeconds={p.freshnessLagSeconds} /> },
              ]}
            />
          </SectionCard>
        </TabsContent>
        <TabsContent value="graph" className="mt-3">
          {p.graph && p.graph.ops.length > 0 ? (
            <FlowCanvas
              nodes={topoSortOps(p.graph.ops, p.graph.edges).map(({ node, upstream }) => ({
                id: node.name,
                label: node.name,
                sublabel: upstream.length > 0 ? `after: ${upstream.join(", ")}` : undefined,
                status: latestRunStepStatus(node.name),
              }))}
            />
          ) : (
            <EmptyState
              title="This pipeline has no graph yet"
              description="The op graph is read from Dagster and is not available for this build."
            />
          )}
        </TabsContent>
        <TabsContent value="source" className="mt-3">
          {p.graph && p.graph.ops.length > 0 ? (
            <SourceTab pipelineId={pipelineId} ops={p.graph.ops} />
          ) : (
            <EmptyState
              title="No source to show"
              description="This pipeline has no op graph yet, so there is nothing to pick an op from."
            />
          )}
        </TabsContent>
        <TabsContent value="runs" className="mt-3">
          {runs.length === 0 ? (
            <EmptyState
              title="No runs"
              description="Runs appear here once the pipeline executes."
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
        <TabsContent value="config" className="mt-3">
          {p.config.length === 0 ? (
            <EmptyState
              title="No configuration recorded"
              description="This pipeline has no recorded run configuration."
            />
          ) : (
            <MetadataList columns={2} items={p.config.map((c) => ({ label: c.key, value: c.value }))} />
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
