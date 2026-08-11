"use client"

import * as React from "react"
import Link from "next/link"
import { useParams } from "next/navigation"
import { PauseIcon, PlayIcon, RotateCcwIcon, SquareIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
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
import { useService, useServiceAction } from "@/hooks/use-service"
import {
  formatCompactNumber,
  formatCost,
  formatDateTime,
  formatDuration,
  formatRelativeTime,
} from "@/lib/format"
import { pipelineService } from "@/services"
import type { PipelineRun } from "@/services/contracts/pipelines"

function runDuration(run: PipelineRun): string {
  if (!run.endedAt) return "running"
  return formatDuration(
    new Date(run.endedAt).getTime() - new Date(run.startedAt).getTime()
  )
}

const runColumns: ColumnDef<PipelineRun>[] = [
  { key: "status", header: "Status", render: (r) => <StatusBadge status={r.status} /> },
  { key: "started", header: "Started", render: (r) => formatRelativeTime(r.startedAt) },
  { key: "duration", header: "Duration", render: (r) => runDuration(r) },
  { key: "processed", header: "Processed", render: (r) => formatCompactNumber(r.processed) },
  { key: "accepted", header: "Accepted", render: (r) => formatCompactNumber(r.accepted) },
  { key: "rejected", header: "Rejected", render: (r) => (
    <span className={r.rejected > 0 ? "text-destructive" : undefined}>
      {formatCompactNumber(r.rejected)}
    </span>
  )},
  { key: "retried", header: "Retried", render: (r) => formatCompactNumber(r.retried) },
  { key: "cost", header: "Cost", render: (r) => formatCost(r.costUnits) },
  { key: "error", header: "Error", render: (r) =>
    r.error ? (
      <span className="block max-w-52 truncate text-destructive" title={r.error}>
        {r.error}
      </span>
    ) : (
      "—"
    ),
  },
]

function AssetLink({ id, label }: { id?: string; label: string }) {
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

function RunDrawer({
  run,
  onClose,
  onChanged,
}: {
  run: PipelineRun | null
  onClose: () => void
  onChanged: () => void
}) {
  const cancelAction = useServiceAction((signal, runId: string) =>
    pipelineService.cancelRun(runId, signal)
  )
  const retryAction = useServiceAction((signal, runId: string) =>
    pipelineService.retryRun(runId, signal)
  )
  const [cancelOpen, setCancelOpen] = React.useState(false)

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
            <div className="flex flex-wrap gap-2">
              {run.status === "running" ? (
                <Button size="sm" variant="outline" onClick={() => setCancelOpen(true)}>
                  <SquareIcon data-icon="inline-start" />
                  Cancel run
                </Button>
              ) : null}
              {run.status === "failed" || run.status === "cancelled" ? (
                <Button
                  size="sm"
                  disabled={retryAction.status === "pending"}
                  onClick={async () => {
                    const next = await retryAction.run(run.id)
                    if (next) {
                      onChanged()
                      onClose()
                    }
                  }}
                >
                  <RotateCcwIcon data-icon="inline-start" />
                  {retryAction.status === "pending" ? "Retrying…" : "Retry run"}
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
            <MetadataList
              items={[
                { label: "Status", value: <StatusBadge status={run.status} /> },
                { label: "Started", value: formatDateTime(run.startedAt) },
                {
                  label: "Ended",
                  value: run.endedAt ? formatDateTime(run.endedAt) : "running",
                },
                { label: "Duration", value: runDuration(run) },
                { label: "Processed", value: formatCompactNumber(run.processed) },
                { label: "Accepted", value: formatCompactNumber(run.accepted) },
                { label: "Rejected", value: formatCompactNumber(run.rejected) },
                { label: "Retried", value: formatCompactNumber(run.retried) },
                { label: "Cost", value: formatCost(run.costUnits) },
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
              ]}
            />
            {run.error ? (
              <div>
                <p className="text-xs font-medium text-muted-foreground">Error</p>
                <p className="mt-1 text-sm text-destructive">{run.error}</p>
              </div>
            ) : null}
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
        onConfirm={async () => {
          if (!run) return
          const updated = await cancelAction.run(run.id)
          if (updated) {
            setCancelOpen(false)
            onChanged()
            onClose()
          }
        }}
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
  const [pauseOpen, setPauseOpen] = React.useState(false)
  const runAction = useServiceAction((signal, id: string) =>
    pipelineService.triggerRun(id, signal)
  )
  const pauseAction = useServiceAction((signal, id: string) =>
    pipelineService.pausePipeline(id, signal)
  )
  const resumeAction = useServiceAction((signal, id: string) =>
    pipelineService.resumePipeline(id, signal)
  )

  if (state.status === "loading") return <LoadingSkeleton rows={8} />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  const p = state.data
  const isPaused = p.status === "paused"

  return (
    <div className="flex flex-col gap-4">
      <EntityHeader
        eyebrow={<Link href="/pipelines" className="hover:underline">Pipelines</Link>}
        title={p.name}
        titleAccessory={<StatusBadge status={p.status} />}
        description={p.description}
        actions={
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
                { label: "Last run", value: formatRelativeTime(p.lastRunAt) },
                { label: "Next run", value: p.nextRunAt ? formatRelativeTime(p.nextRunAt) : "—" },
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
          <FlowCanvas
            nodes={p.graph.map((n) => ({
              id: n.id,
              label: n.label,
              kind: n.kind,
              status: n.status,
            }))}
          />
        </TabsContent>
        <TabsContent value="runs" className="mt-3">
          {p.runs.length === 0 ? (
            <EmptyState
              title="No runs yet"
              description="Runs appear here once the pipeline executes."
            />
          ) : (
            <DataTable
              columns={runColumns}
              rows={p.runs}
              rowKey={(r) => r.id}
              onRowClick={setSelectedRun}
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
