"use client"

import { formatCompactNumber, formatDuration, parseTimestamp } from "@/lib/format"
import { fmtMeasured } from "@/lib/measured"
import { cn } from "@/lib/utils"
import type { PipelineRun, PipelineRunStep } from "@/services/contracts/pipelines"
import { runFill, statusLabel } from "./run-history-strip"

function ts(iso: string | null | undefined): number | null {
  if (!iso) return null
  const t = parseTimestamp(iso).getTime()
  return Number.isNaN(t) ? null : t
}

/**
 * A run's steps on one time axis (the Gantt the Dagster and Kestra run
 * pages lead with). The top row is the run itself: a hatched segment for
 * the time it spent queued and launching (`queuedAt` → `startedAt`),
 * then the run. A step the orchestrator reported no times for is listed
 * with "no timing". It is never drawn at a guessed position. Clicking a
 * step row filters the log below to that step.
 */
export function StepTimeline({
  run,
  steps,
  now,
  activeStep,
  onToggleStep,
}: {
  run: PipelineRun
  steps: readonly PipelineRunStep[]
  now: number
  activeStep: string | null
  onToggleStep: (stepKey: string) => void
}) {
  const queued = ts(run.queuedAt)
  const started = ts(run.startedAt)
  const ended = ts(run.endedAt) ?? (run.status === "running" ? now : null)
  const times = [
    queued,
    started,
    ended,
    ...steps.flatMap((s) => [s.startMs, s.endMs ?? (s.startMs !== null && s.status === "running" ? now : null)]),
  ].filter((t): t is number => t !== null)
  if (times.length < 2) {
    return <p className="text-xs text-muted-foreground">The orchestrator reported no timing for this run yet.</p>
  }
  const origin = Math.min(...times)
  const span = Math.max(1, Math.max(...times) - origin)
  const pct = (t: number) => ((t - origin) / span) * 100
  const ticks = [0, 0.25, 0.5, 0.75, 1]

  return (
    <div className="flex flex-col gap-1.5">
      <div className="grid grid-cols-[minmax(0,12rem)_1fr_4.5rem] items-end gap-3 px-1.5 pb-1">
        <span />
        <div className="relative h-4">
          {ticks.map((f) => (
            <span
              key={f}
              className={cn(
                "absolute font-mono text-[10px] text-muted-foreground",
                f === 0 ? "left-0" : f === 1 ? "right-0" : "-translate-x-1/2"
              )}
              style={f > 0 && f < 1 ? { left: `${f * 100}%` } : undefined}
            >
              {f === 0 ? "0" : `+${formatDuration(f * span)}`}
            </span>
          ))}
        </div>
        <span />
      </div>

      <TimelineRow
        label="Run"
        sub={statusLabel(run.status)}
        track={
          <>
            {queued !== null && started !== null && started > queued ? (
              <span
                title={`Queued and launching: ${formatDuration(started - queued)}`}
                className="absolute inset-y-1 rounded-sm bg-[repeating-linear-gradient(135deg,var(--border)_0_4px,transparent_4px_8px)]"
                style={{ left: `${pct(queued)}%`, width: `${pct(started) - pct(queued)}%` }}
              />
            ) : null}
            {started !== null && ended !== null ? (
              <span
                className={cn("absolute inset-y-1 rounded-sm opacity-30", runFill(run.status))}
                style={{ left: `${pct(started)}%`, width: `max(3px, ${pct(ended) - pct(started)}%)` }}
              />
            ) : null}
          </>
        }
        value={started !== null && ended !== null ? formatDuration(ended - started) : "—"}
      />

      {steps.map((step) => {
        const end = step.endMs ?? (step.status === "running" ? now : null)
        const rows = step.materializations.find((m) => m.rows !== null)?.rows ?? null
        const active = activeStep === step.stepKey
        return (
          <TimelineRow
            key={step.stepKey}
            label={step.stepKey}
            sub={rows !== null ? `${fmtMeasured(rows, formatCompactNumber)} rows` : statusLabel(step.status)}
            active={active}
            onClick={() => onToggleStep(step.stepKey)}
            track={
              step.startMs !== null && end !== null ? (
                <span
                  title={`${step.stepKey}: ${statusLabel(step.status)}, ${formatDuration(end - step.startMs)}`}
                  className={cn(
                    "absolute inset-y-1 rounded-sm",
                    runFill(step.status),
                    step.status === "running" && "animate-pulse"
                  )}
                  style={{ left: `${pct(step.startMs)}%`, width: `max(3px, ${pct(end) - pct(step.startMs)}%)` }}
                />
              ) : (
                <span className="absolute inset-y-0 left-2 flex items-center text-[10px] text-muted-foreground">
                  no timing
                </span>
              )
            }
            value={step.startMs !== null && end !== null ? formatDuration(end - step.startMs) : "—"}
          />
        )
      })}
    </div>
  )
}

function TimelineRow({
  label,
  sub,
  track,
  value,
  active = false,
  onClick,
}: {
  label: string
  sub: string
  track: React.ReactNode
  value: string
  active?: boolean
  onClick?: () => void
}) {
  const body = (
    <>
      <span className="min-w-0 text-left">
        <span className="block truncate font-mono text-xs text-foreground">{label}</span>
        <span className="block truncate text-[11px] text-muted-foreground">{sub}</span>
      </span>
      <span className="relative h-7 overflow-hidden rounded-md bg-muted/50 bg-[linear-gradient(to_right,var(--border)_1px,transparent_1px)] bg-[length:25%_100%]">
        {track}
      </span>
      <span className="text-right font-mono text-xs tabular-nums text-muted-foreground">{value}</span>
    </>
  )
  const grid = "grid grid-cols-[minmax(0,12rem)_1fr_4.5rem] items-center gap-3 rounded-lg px-1.5 py-1"
  if (!onClick) return <div className={grid}>{body}</div>
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      title={active ? "Show every step's log lines" : "Show only this step's log lines"}
      className={cn(grid, "transition-colors hover:bg-muted/60", active && "bg-primary/8 ring-1 ring-primary/30")}
    >
      {body}
    </button>
  )
}
