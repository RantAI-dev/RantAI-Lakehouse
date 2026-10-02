"use client"

import { CircleAlertIcon, CircleCheckIcon, CircleDashedIcon, CircleSlashIcon } from "lucide-react"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService } from "@/hooks/use-service"
import { formatDateTime, formatRelativeTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { pipelineService } from "@/services"
import type { ScheduleTick } from "@/services/contracts/pipelines"

function TickIcon({ tick }: { tick: ScheduleTick }) {
  const cls = "size-4 shrink-0"
  if (tick.status === "SUCCESS") return <CircleCheckIcon className={cn(cls, "text-emerald-500")} aria-hidden />
  if (tick.status === "SKIPPED") return <CircleSlashIcon className={cn(cls, "text-muted-foreground")} aria-hidden />
  if (tick.status === "FAILURE" || tick.failed) return <CircleAlertIcon className={cn(cls, "text-destructive")} aria-hidden />
  return <CircleDashedIcon className={cn(cls, "text-sky-500")} aria-hidden />
}

function tickText(tick: ScheduleTick): string {
  if (tick.status === "SUCCESS") {
    return tick.runIds.length === 1 ? "Launched a run" : `Launched ${tick.runIds.length} runs`
  }
  if (tick.status === "SKIPPED") return tick.skipReason ? `Skipped: ${tick.skipReason}` : "Skipped"
  if (tick.status === "FAILURE" || tick.failed) {
    // The orchestrator's error text is not sent to the console.
    return "Failed to evaluate; the reason is in the orchestrator's tick log"
  }
  return "Evaluating"
}

/**
 * The schedule's recent ticks: each time it was due, and whether it
 * launched a run, was skipped, or failed. This is what the run history
 * cannot show: a schedule that fired and launched nothing leaves no run.
 */
export function ScheduleHistory({
  pipelineId,
  hasSchedule,
  onSelectRun,
}: {
  pipelineId: string
  hasSchedule: boolean
  onSelectRun: (runId: string) => void
}) {
  const ticks = useService((s) => pipelineService.getScheduleTicks(pipelineId, s), [pipelineId])
  return (
    <section className="mt-4 rounded-xl border border-border bg-card p-4">
      <h3 className="mb-3 font-mono text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
        Schedule history
      </h3>
      {!hasSchedule ? (
        <p className="text-sm text-muted-foreground">This pipeline has no schedule; it runs only when launched.</p>
      ) : ticks.status === "loading" ? (
        <LoadingSkeleton rows={3} />
      ) : ticks.status === "error" ? (
        <ErrorState error={ticks.error} onRetry={ticks.reload} />
      ) : ticks.data.unavailable ? (
        <p className="text-sm text-muted-foreground">The orchestrator could not be reached for the schedule&apos;s history.</p>
      ) : ticks.data.schedule === null || ticks.data.ticks.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          The schedule has not been due yet{ticks.data.schedule ? "" : ", or the orchestrator has not built it"}.
        </p>
      ) : (
        <>
          <p className="mb-2 text-xs text-muted-foreground">
            <span className="font-mono">{ticks.data.schedule}</span> · last {ticks.data.ticks.length} ticks
          </p>
          <ol className="flex flex-col divide-y divide-border">
            {ticks.data.ticks.map((tick) => {
              const iso = new Date(tick.timestamp * 1000).toISOString()
              return (
                <li key={tick.tickId} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2 text-sm">
                  <TickIcon tick={tick} />
                  <span className="w-44 shrink-0 text-muted-foreground" title={formatDateTime(iso)}>
                    {formatDateTime(iso)} · {formatRelativeTime(iso)}
                  </span>
                  <span className="min-w-0 flex-1">{tickText(tick)}</span>
                  {tick.runIds.map((id) => (
                    <button
                      key={id}
                      type="button"
                      onClick={() => onSelectRun(id)}
                      className="font-mono text-xs text-primary hover:underline"
                    >
                      {id.slice(0, 8)}
                    </button>
                  ))}
                </li>
              )
            })}
          </ol>
        </>
      )}
    </section>
  )
}
