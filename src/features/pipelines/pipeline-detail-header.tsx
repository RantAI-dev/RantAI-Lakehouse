"use client"

import Link from "next/link"
import { CalendarClockIcon, ChevronRightIcon, CpuIcon, HandIcon, PenToolIcon } from "lucide-react"
import { Pill, StatusBadge } from "@/components/patterns/status-badge"
import { BrandBackdrop } from "@/components/ui/brand-glow"
import { formatDateTime, formatDuration, formatPercent, formatRelativeTime, isPast } from "@/lib/format"
import { ENTITY_STATUS_LABEL, type EntityStatus } from "@/lib/status"
import { cn } from "@/lib/utils"
import type { PipelineDetail, PipelineRun } from "@/services/contracts/pipelines"
import type { RunSummary } from "./pipeline-run-stats"
import { runDurationMs } from "./pipeline-run-stats"
import { describeCron, localizeScheduleTime, type ParsedSchedule } from "./pipeline-schedule"

/** A status the shared badge knows, or `null` for what the API sends outside it ("unknown" for a never-run job). */
export function knownStatus(status: string): EntityStatus | null {
  return status in ENTITY_STATUS_LABEL ? (status as EntityStatus) : null
}

function ScheduleChip({
  schedule,
  paused,
  nextRunAt,
}: {
  schedule: ParsedSchedule
  paused: boolean
  nextRunAt: string | null | undefined
}) {
  if (schedule.kind === "manual") {
    return (
      <Pill tone="neutral">
        <HandIcon className="size-3" aria-hidden />
        Manual only
      </Pill>
    )
  }
  const cron = schedule.kind === "cron" ? schedule.cron : schedule.raw
  const described = schedule.kind === "cron" ? describeCron(schedule.cron) : null
  const words = described ? localizeScheduleTime(described, nextRunAt) : null
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs",
        paused ? "border-dashed border-border text-muted-foreground" : "border-primary/25 bg-primary/5 text-foreground"
      )}
      title={`cron ${cron}, in the schedule's own timezone${words && nextRunAt ? "; the time shown is your local time" : ""}`}
    >
      <CalendarClockIcon className="size-3.5 text-primary" aria-hidden />
      {words ?? "Cron"}
      <code className="font-mono text-[11px] text-muted-foreground">{cron}</code>
      {paused ? <span className="font-medium text-amber-600 dark:text-amber-400">· paused</span> : null}
    </span>
  )
}

/**
 * The page's masthead: where the pipeline sits, what it is, how it is
 * triggered, and its actions. The health badge is the newest run's status,
 * which is what `routes::pipelines` derives a Dagster job's status from;
 * a job that has never run says so instead of "unknown".
 */
export function PipelineMasthead({
  pipeline,
  latest,
  schedule,
  schedulePaused,
  actions,
}: {
  pipeline: PipelineDetail
  latest: PipelineRun | null
  schedule: ParsedSchedule
  schedulePaused: boolean
  actions: React.ReactNode
}) {
  const authored = pipeline.engine === "authored"
  const health = latest ? latest.status : knownStatus(pipeline.status)
  return (
    <header className="relative isolate overflow-hidden rounded-2xl border border-border bg-card/60 px-5 pb-5 pt-4">
      <BrandBackdrop className="opacity-60" />
      <nav className="flex items-center gap-1 text-xs text-muted-foreground" aria-label="Breadcrumb">
        <Link href="/pipelines" className="hover:text-foreground hover:underline">
          Pipelines
        </Link>
        <ChevronRightIcon className="size-3" aria-hidden />
        <span className="font-mono">{pipeline.id}</span>
      </nav>
      <div className="mt-3 flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2.5">
            <h1 className="truncate text-[1.7rem] font-semibold leading-9 tracking-[-0.025em]">{pipeline.name}</h1>
            {health ? (
              <StatusBadge status={health} />
            ) : (
              <Pill tone="neutral">{pipeline.lastRunAt === null ? "Never run" : "Unknown"}</Pill>
            )}
          </div>
          {pipeline.description ? (
            <p className="mt-1 max-w-3xl text-sm leading-6 text-muted-foreground">{pipeline.description}</p>
          ) : null}
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Pill tone={authored ? "violet" : "sky"}>
              {authored ? <PenToolIcon className="size-3" aria-hidden /> : <CpuIcon className="size-3" aria-hidden />}
              {authored ? "Authored pipeline" : "Dagster job"}
            </Pill>
            <ScheduleChip schedule={schedule} paused={schedulePaused} nextRunAt={pipeline.nextRunAt} />
            <span className="text-xs text-muted-foreground">
              Owner <span className="font-medium text-foreground">{pipeline.owner}</span>
            </span>
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>
      </div>
    </header>
  )
}

function Vital({
  label,
  children,
  foot,
  delay,
}: {
  label: string
  children: React.ReactNode
  foot?: React.ReactNode
  delay: number
}) {
  return (
    <div
      className="flex min-w-0 flex-col gap-1 px-4 py-3 motion-safe:animate-[vital-in_0.5s_cubic-bezier(0.2,0.8,0.2,1)_both]"
      style={{ animationDelay: `${delay}ms` }}
    >
      <span className="font-mono text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground">{label}</span>
      <div className="flex min-h-7 items-baseline gap-2">{children}</div>
      {foot ? <div className="truncate text-xs text-muted-foreground">{foot}</div> : null}
    </div>
  )
}

/**
 * Four figures computed from the run window the API returned (the last 30
 * runs): the newest run, the success rate of finished runs, median and
 * p95 wall time, and the next scheduled run. A figure with no input reads
 * "—" with the reason, never a zero.
 */
export function PipelineVitals({
  summary,
  latest,
  nextRunAt,
  schedule,
  schedulePaused,
  now,
}: {
  summary: RunSummary
  latest: PipelineRun | null
  nextRunAt: string | null | undefined
  schedule: ParsedSchedule
  schedulePaused: boolean
  now: number
}) {
  const latestMs = latest ? runDurationMs(latest, now) : null
  const rate = summary.successRate
  return (
    <div className="grid grid-cols-2 gap-px overflow-hidden rounded-xl border border-border bg-border lg:grid-cols-4 [&>*]:bg-card">
      <Vital label="Last run" delay={0} foot={latest ? `${formatDateTime(latest.startedAt)}` : "No run in the orchestrator's history"}>
        {latest ? (
          <>
            <StatusBadge status={latest.status} />
            <span className="text-lg font-semibold tracking-tight">{formatRelativeTime(latest.startedAt, now)}</span>
            {latestMs !== null ? (
              <span className="font-mono text-xs tabular-nums text-muted-foreground">{formatDuration(latestMs)}</span>
            ) : null}
          </>
        ) : (
          <span className="text-lg font-semibold text-muted-foreground">—</span>
        )}
      </Vital>
      <Vital
        label="Success rate"
        delay={60}
        foot={
          rate === null
            ? "No finished run yet"
            : `${summary.succeeded} of ${summary.finished} finished · last ${summary.total} runs`
        }
      >
        {rate === null ? (
          <span className="text-lg font-semibold text-muted-foreground">—</span>
        ) : (
          <>
            <span
              className={cn(
                "font-mono text-2xl font-semibold tabular-nums tracking-tight",
                rate >= 0.95 ? "text-emerald-600 dark:text-emerald-400" : rate >= 0.7 ? "text-amber-600 dark:text-amber-400" : "text-destructive"
              )}
            >
              {formatPercent(rate)}
            </span>
            <span className="flex h-1.5 w-full max-w-24 self-center overflow-hidden rounded-full bg-destructive/25" aria-hidden>
              <span className="h-full bg-emerald-500" style={{ width: `${rate * 100}%` }} />
            </span>
          </>
        )}
      </Vital>
      <Vital
        label="Duration"
        delay={120}
        foot={summary.p95Ms !== null ? `p95 ${formatDuration(summary.p95Ms)} · completed runs` : "No completed run to measure"}
      >
        <span className="font-mono text-2xl font-semibold tabular-nums tracking-tight">
          {summary.p50Ms !== null ? formatDuration(summary.p50Ms) : "—"}
        </span>
        {summary.p50Ms !== null ? <span className="text-xs text-muted-foreground">median</span> : null}
      </Vital>
      <Vital
        label="Next run"
        delay={180}
        foot={
          schedule.kind === "manual"
            ? "Runs only when launched"
            : schedulePaused
              ? "Schedule paused: nothing will fire"
              : nextRunAt
                ? formatDateTime(nextRunAt)
                : "The orchestrator gave no next time"
        }
      >
        {schedule.kind === "manual" || schedulePaused || !nextRunAt ? (
          <span className="text-lg font-semibold text-muted-foreground">—</span>
        ) : isPast(nextRunAt, now) ? (
          <span className="text-lg font-semibold text-amber-600 dark:text-amber-400">Overdue</span>
        ) : (
          <span className="text-lg font-semibold tracking-tight">in {formatCountdown(Date.parse(nextRunAt) - now)}</span>
        )}
      </Vital>
    </div>
  )
}

/** "4h 12m", "38m", "45s": time until an event. */
function formatCountdown(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000))
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  if (h < 48) return `${h}h ${String(m % 60).padStart(2, "0")}m`
  return `${Math.floor(h / 24)}d`
}
