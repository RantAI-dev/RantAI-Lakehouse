"use client"

import * as React from "react"
import Link from "next/link"
import {
  CalendarClockIcon,
  ChevronDownIcon,
  CopyIcon,
  GitForkIcon,
  HandIcon,
  HistoryIcon,
  OctagonAlertIcon,
  RadarIcon,
  RotateCcwIcon,
  SquareIcon,
} from "lucide-react"
import { toast } from "sonner"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatDateTime, formatDuration, formatNumber, parseTimestamp } from "@/lib/format"
import { withNotify } from "@/lib/notify"
import { pipelineService } from "@/services"
import type { PipelineRun, PipelineRunTrigger, RetryStrategy } from "@/services/contracts/pipelines"
import { isLiveRun, runDurationMs } from "./pipeline-run-stats"
import { RunLogConsole, useRunLogs } from "./run-log-console"
import { failureLines, type LogLevelFilter } from "./run-logs"
import { StepTimeline } from "./step-timeline"

function TriggerChip({
  trigger,
  onSelectRun,
  parentRunId,
}: {
  trigger: PipelineRunTrigger | undefined
  parentRunId: string | null | undefined
  onSelectRun: (id: string) => void
}) {
  if (!trigger) return null
  const base =
    "inline-flex items-center gap-1.5 rounded-full border border-border bg-background/70 px-2 py-0.5 text-xs text-muted-foreground"
  switch (trigger.kind) {
    case "schedule":
      return (
        <span className={base} title="Launched by the orchestrator's schedule">
          <CalendarClockIcon className="size-3.5" aria-hidden />
          Schedule{trigger.name ? <span className="font-mono">· {trigger.name}</span> : null}
        </span>
      )
    case "sensor":
      return (
        <span className={base} title="Launched by a sensor">
          <RadarIcon className="size-3.5" aria-hidden />
          Sensor{trigger.name ? <span className="font-mono">· {trigger.name}</span> : null}
        </span>
      )
    case "retry":
      return (
        <button
          type="button"
          className={`${base} hover:border-primary/40 hover:text-foreground`}
          onClick={() => parentRunId && onSelectRun(parentRunId)}
          title="Open the run this one re-executes"
        >
          <RotateCcwIcon className="size-3.5" aria-hidden />
          Retry of <span className="font-mono">{parentRunId?.slice(0, 8)}</span>
        </button>
      )
    case "backfill":
      return (
        <span className={base}>
          <HistoryIcon className="size-3.5" aria-hidden />
          Backfill{trigger.name ? <span className="font-mono">· {trigger.name}</span> : null}
        </span>
      )
    default:
      return (
        <span className={base} title="No schedule, sensor or backfill launched it: started from the console, the Copilot or the API">
          <HandIcon className="size-3.5" aria-hidden />
          Manual
        </span>
      )
  }
}

function FailureCallout({
  lines,
  loading,
  onShowErrors,
}: {
  lines: ReturnType<typeof failureLines>
  loading: boolean
  onShowErrors: () => void
}) {
  const step = lines.find((l) => l.stepKey)?.stepKey
  return (
    <div className="relative overflow-hidden rounded-xl border border-destructive/30 bg-destructive/[0.06] p-4">
      <div aria-hidden className="absolute inset-y-0 left-0 w-1 bg-destructive" />
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-semibold text-destructive">
          <OctagonAlertIcon className="size-4" aria-hidden />
          {step ? (
            <span>
              Failed in <span className="font-mono">{step}</span>
            </span>
          ) : (
            <span>Run failed</span>
          )}
        </div>
        {lines.length > 0 ? (
          <Button size="xs" variant="ghost" onClick={onShowErrors}>
            Show errors in the log
          </Button>
        ) : null}
      </div>
      {loading && lines.length === 0 ? (
        <p className="mt-2 text-xs text-muted-foreground">Reading the log for the error…</p>
      ) : lines.length === 0 ? (
        <p className="mt-2 text-xs text-muted-foreground">
          The run failed, but its log holds no error line. The orchestrator recorded no reason.
        </p>
      ) : (
        <ul className="mt-2 flex flex-col gap-1.5">
          {lines.map((l, i) => (
            <li key={`${l.ts}-${i}`} className="whitespace-pre-wrap break-words font-mono text-xs leading-5 text-foreground">
              {l.message}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

/**
 * One run, inspected: what launched it, how long it queued and ran, why it
 * failed (the log's own error lines, since the run record carries no error
 * text), its steps on a time axis, and the full log. Re-run offers both
 * strategies the orchestrator has: every step, or only the failed ones.
 */
export function RunInspector({
  run,
  now,
  onSelectRun,
  onChanged,
}: {
  run: PipelineRun
  now: number
  onSelectRun: (id: string) => void
  onChanged: (focusNewest: boolean) => void
}) {
  const live = isLiveRun(run)
  const steps = useService(
    (s) => pipelineService.getRunSteps(run.pipelineId, run.id, s),
    [run.pipelineId, run.id, run.status],
    { keepDataOnReload: true }
  )
  // Steps change while a run is live; re-read them with the log's cadence.
  const reloadSteps = steps.reload
  React.useEffect(() => {
    if (!live) return
    const t = setInterval(reloadSteps, 4000)
    return () => clearInterval(t)
  }, [live, reloadSteps])

  const logs = useRunLogs(run.pipelineId, run.id, live)
  const [level, setLevel] = React.useState<LogLevelFilter>("info")
  const [stepKey, setStepKey] = React.useState<string | null>(null)
  React.useEffect(() => {
    setStepKey(null)
    setLevel("info")
  }, [run.id])

  const errors = React.useMemo(() => failureLines(logs.lines), [logs.lines])
  const logRef = React.useRef<HTMLDivElement>(null)

  const retry = useServiceAction(
    withNotify(
      { success: "Re-run launched", error: "The re-run could not be launched" },
      (signal, strategy: RetryStrategy) => pipelineService.retryRun(run.id, signal, strategy)
    )
  )
  const cancel = useServiceAction(
    withNotify(
      { success: "Run cancelled", error: "The run could not be cancelled" },
      (signal) => pipelineService.cancelRun(run.id, signal)
    )
  )
  const [cancelOpen, setCancelOpen] = React.useState(false)

  const started = parseTimestamp(run.startedAt).getTime()
  const queued = run.queuedAt ? parseTimestamp(run.queuedAt).getTime() : null
  const duration = runDurationMs(run, now)
  const canRetry = run.status === "failed" || run.status === "cancelled"
  // Rows the run's ops reported writing (step materializations). Absent
  // when no op reported a count: an unmeasured run shows no figure, not 0.
  const rowsWritten = React.useMemo(() => {
    if (steps.status !== "success") return null
    const counts = steps.data.flatMap((s) =>
      s.materializations.map((m) => m.rows).filter((r): r is number => typeof r === "number")
    )
    return counts.length > 0 ? counts.reduce((a, b) => a + b, 0) : null
  }, [steps])

  const relaunch = async (strategy: RetryStrategy) => {
    if (await retry.run(strategy)) onChanged(true)
  }

  return (
    <div className="flex min-w-0 flex-col gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="font-mono text-base font-semibold tracking-tight">
              Run {run.id.slice(0, 8)}
            </h3>
            <button
              type="button"
              className="rounded p-0.5 text-muted-foreground hover:text-foreground"
              title="Copy the full run id"
              aria-label="Copy the full run id"
              onClick={async () => {
                try {
                  await navigator.clipboard.writeText(run.id)
                  toast.success("Run id copied")
                } catch {
                  toast.error("The browser refused clipboard access")
                }
              }}
            >
              <CopyIcon className="size-3.5" />
            </button>
            <StatusBadge status={run.status} />
            <TriggerChip trigger={run.trigger} parentRunId={run.parentRunId} onSelectRun={onSelectRun} />
          </div>
          <dl className="mt-2 flex flex-wrap gap-x-5 gap-y-1 text-xs text-muted-foreground">
            <div className="flex gap-1.5">
              <dt>Started</dt>
              <dd className="font-medium text-foreground">{Number.isNaN(started) ? "—" : formatDateTime(run.startedAt)}</dd>
            </div>
            <div className="flex gap-1.5">
              <dt>{live ? "Running for" : "Took"}</dt>
              <dd className="font-mono font-medium tabular-nums text-foreground">
                {duration === null ? "—" : formatDuration(duration)}
              </dd>
            </div>
            {rowsWritten !== null ? (
              <div className="flex gap-1.5" title="Rows the run's ops reported writing">
                <dt>Rows written</dt>
                <dd className="font-mono font-medium tabular-nums text-foreground">{formatNumber(rowsWritten)}</dd>
              </div>
            ) : null}
            {queued !== null && !Number.isNaN(started) && started > queued ? (
              <div className="flex gap-1.5" title="From the run's creation to its start: waiting in the queue and launching its process">
                <dt>Queued</dt>
                <dd className="font-mono font-medium tabular-nums text-foreground">{formatDuration(started - queued)}</dd>
              </div>
            ) : null}
          </dl>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {live ? (
            <Button size="sm" variant="outline" onClick={() => setCancelOpen(true)}>
              <SquareIcon data-icon="inline-start" />
              Cancel run
            </Button>
          ) : null}
          {canRetry ? (
            run.status === "failed" ? (
              <DropdownMenu>
                <DropdownMenuTrigger
                  render={
                    <Button size="sm" disabled={retry.status === "pending"}>
                      <RotateCcwIcon data-icon="inline-start" />
                      {retry.status === "pending" ? "Launching…" : "Re-run"}
                      <ChevronDownIcon data-icon="inline-end" />
                    </Button>
                  }
                />
                <DropdownMenuContent align="end" className="w-64">
                  <DropdownMenuItem onClick={() => relaunch("fromFailure")}>
                    <div className="flex flex-col">
                      <span className="font-medium">From failure</span>
                      <span className="text-xs text-muted-foreground">Only the steps that failed or never ran</span>
                    </div>
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => relaunch("allSteps")}>
                    <div className="flex flex-col">
                      <span className="font-medium">All steps</span>
                      <span className="text-xs text-muted-foreground">The whole run again, from the start</span>
                    </div>
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            ) : (
              <Button size="sm" disabled={retry.status === "pending"} onClick={() => relaunch("allSteps")}>
                <RotateCcwIcon data-icon="inline-start" />
                {retry.status === "pending" ? "Launching…" : "Re-run"}
              </Button>
            )
          ) : null}
          <Button size="sm" variant="ghost" render={<Link href={`/lineage?focus=${encodeURIComponent(run.pipelineId)}`} />}>
            <GitForkIcon data-icon="inline-start" />
            Lineage
          </Button>
        </div>
      </div>

      {run.status === "failed" ? (
        <FailureCallout
          lines={errors}
          loading={logs.loading}
          onShowErrors={() => {
            setStepKey(null)
            setLevel("error")
            logRef.current?.scrollIntoView({ behavior: "smooth", block: "start" })
          }}
        />
      ) : null}

      <section className="flex flex-col gap-2">
        <h4 className="font-mono text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
          Step timeline
        </h4>
        {steps.status === "loading" ? <LoadingSkeleton rows={2} /> : null}
        {steps.status === "error" ? <ErrorState error={steps.error} onRetry={steps.reload} /> : null}
        {steps.status === "success" ? (
          <StepTimeline
            run={run}
            steps={steps.data}
            now={now}
            activeStep={stepKey}
            onToggleStep={(key) => setStepKey((cur) => (cur === key ? null : key))}
          />
        ) : null}
      </section>

      <section ref={logRef} className="flex scroll-mt-4 flex-col gap-2">
        <h4 className="font-mono text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
          Event log
        </h4>
        <RunLogConsole
          logs={logs}
          originMs={Number.isNaN(started) ? (logs.lines[0]?.ts ?? 0) : started}
          level={level}
          onLevel={setLevel}
          stepKey={stepKey}
          onClearStep={() => setStepKey(null)}
          live={live}
          fileName={`${run.pipelineId}-${run.id.slice(0, 8)}.log`}
        />
      </section>

      <ConfirmActionDialog
        open={cancelOpen}
        onOpenChange={setCancelOpen}
        title="Cancel pipeline run"
        description={`Cancel run ${run.id.slice(0, 8)}?`}
        impact="In-flight work stops at the last checkpoint. Partial output may remain."
        confirmLabel="Cancel run"
        confirming={cancel.status === "pending"}
        onConfirm={async () => {
          if (await cancel.run()) {
            setCancelOpen(false)
            onChanged(false)
          }
        }}
      />
    </div>
  )
}
