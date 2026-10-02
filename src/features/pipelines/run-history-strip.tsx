"use client"

import * as React from "react"
import { motion, useReducedMotion } from "motion/react"
import { formatDateTime, formatDuration, formatRelativeTime } from "@/lib/format"
import { ENTITY_STATUS_LABEL, type EntityStatus } from "@/lib/status"
import { cn } from "@/lib/utils"
import type { PipelineRun } from "@/services/contracts/pipelines"
import { isLiveRun, runDurationMs } from "./pipeline-run-stats"

/**
 * Fill colour per run status, shared by the history strip, the runs list
 * and the step timeline so one status reads the same everywhere. The
 * status is always also written out (tooltip, badge, label), so colour is
 * never the only cue.
 */
export const RUN_FILL: Partial<Record<EntityStatus, string>> = {
  completed: "bg-emerald-500",
  failed: "bg-destructive",
  running: "bg-primary",
  scheduled: "bg-sky-400",
  validating: "bg-sky-400",
  cancelled: "bg-muted-foreground/40",
  partial: "bg-amber-500",
  degraded: "bg-amber-500",
  blocked: "bg-amber-500",
}

export function runFill(status: EntityStatus): string {
  return RUN_FILL[status] ?? "bg-muted-foreground/40"
}

export function statusLabel(status: string): string {
  return ENTITY_STATUS_LABEL[status as EntityStatus] ?? status
}

const WINDOW = 30

/**
 * The last runs as a row of bars, oldest on the left: height is wall time
 * (square-root scaled, so one slow run does not flatten the rest), colour
 * is status. The dashed line is the median of completed runs. Unused slots
 * in the window are drawn as faint ticks, so "3 runs" does not look like a
 * full history. Clicking a bar selects that run.
 */
export function RunHistoryStrip({
  runs,
  selectedId,
  onSelect,
  p50Ms,
}: {
  /** Newest first, as the API returns them. */
  runs: readonly PipelineRun[]
  selectedId: string | null
  onSelect: (run: PipelineRun) => void
  p50Ms: number | null
}) {
  const reduce = useReducedMotion() ?? false
  const ordered = React.useMemo(() => [...runs].slice(0, WINDOW).reverse(), [runs])
  const durations = ordered.map((r) => runDurationMs(r))
  const max = Math.max(1, ...durations.map((d) => d ?? 0))
  const scale = (ms: number) => Math.max(0.06, Math.sqrt(ms / max))
  const empty = Math.max(0, WINDOW - ordered.length)

  return (
    <div className="relative">
      <div className="relative flex h-28 items-end gap-[3px]" role="group" aria-label="Run history">
        {Array.from({ length: empty }, (_, i) => (
          <div key={`empty-${i}`} className="flex h-full flex-1 items-end" aria-hidden>
            <div className="h-1 w-full rounded-full bg-border/70" />
          </div>
        ))}
        {ordered.map((run, i) => {
          const d = durations[i]
          const live = isLiveRun(run)
          const selected = run.id === selectedId
          const height = d === null ? 0.15 : scale(d)
          const label = `${statusLabel(run.status)} · ${formatDateTime(run.startedAt)} · ${
            d === null ? "duration unknown" : formatDuration(d)
          }${run.trigger ? ` · ${run.trigger.kind}` : ""}`
          return (
            <button
              key={run.id}
              type="button"
              title={label}
              aria-label={label}
              aria-pressed={selected}
              onClick={() => onSelect(run)}
              className="group relative flex h-full flex-1 items-end rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
            >
              <span
                aria-hidden
                className={cn(
                  "absolute inset-x-0 bottom-0 top-0 rounded-sm transition-colors",
                  selected ? "bg-primary/8" : "group-hover:bg-muted/60"
                )}
              />
              <motion.span
                aria-hidden
                initial={reduce ? false : { scaleY: 0 }}
                animate={{ scaleY: 1 }}
                transition={{ duration: 0.45, delay: reduce ? 0 : i * 0.015, ease: [0.2, 0.8, 0.2, 1] }}
                style={{ height: `${height * 100}%`, originY: 1 }}
                className={cn(
                  "relative w-full rounded-[3px] transition-[opacity,box-shadow]",
                  runFill(run.status),
                  selected
                    ? "opacity-100 shadow-[0_0_0_2px_var(--background),0_0_0_3.5px_var(--primary),0_6px_18px_-4px_var(--primary)]"
                    : "opacity-70 group-hover:opacity-100",
                  live && "animate-pulse"
                )}
              />
            </button>
          )
        })}
        {p50Ms !== null ? (
          <div
            aria-hidden
            className="pointer-events-none absolute inset-x-0 border-t border-dashed border-foreground/25"
            style={{ bottom: `${scale(p50Ms) * 100}%` }}
          >
            <span className="absolute left-0 top-0 -translate-y-full rounded bg-card/90 px-1 font-mono text-[10px] text-muted-foreground">
              p50 {formatDuration(p50Ms)}
            </span>
          </div>
        ) : null}
      </div>
      <div className="mt-2 flex justify-between font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
        <span>{ordered.length > 0 ? formatRelativeTime(ordered[0].startedAt) : ""}</span>
        <span>
          {ordered.length} of last {WINDOW}
        </span>
        <span>{ordered.length > 0 ? formatRelativeTime(ordered[ordered.length - 1].startedAt) : ""}</span>
      </div>
    </div>
  )
}
