/**
 * Run-history arithmetic for the pipeline detail page.
 *
 * Every figure is computed from the runs `GET /api/pipelines/{id}/runs`
 * returned (the 30 most recent). Nothing is extrapolated past that window,
 * and the page labels it "last N runs". A figure with no input, such as a
 * success rate before any run has finished, is `null`, never 0.
 */
import { parseTimestamp } from "@/lib/format"
import type { EntityStatus } from "@/lib/status"
import type { PipelineRun } from "@/services/contracts/pipelines"

/** Statuses a run can still leave. Polling continues while any run is in one. */
const LIVE: ReadonlySet<EntityStatus> = new Set<EntityStatus>([
  "running",
  "scheduled",
  "validating",
])

export function isLiveRun(run: Pick<PipelineRun, "status">): boolean {
  return LIVE.has(run.status)
}

function ms(iso: string | undefined | null): number | null {
  if (!iso) return null
  const t = parseTimestamp(iso).getTime()
  return Number.isNaN(t) ? null : t
}

/**
 * Wall time of a run in milliseconds. A live run is measured up to `now`;
 * a finished run is the gap between its two timestamps, which are exact to
 * the millisecond. `durationSeconds` is the same gap rounded to a whole
 * second, so it is only the fallback when a timestamp is missing, and the
 * header and the step timeline never disagree. `null` when neither is
 * known.
 */
export function runDurationMs(run: PipelineRun, now = Date.now()): number | null {
  const start = ms(run.startedAt)
  const end = ms(run.endedAt)
  if (start !== null && end !== null) return Math.max(0, end - start)
  if (start !== null && isLiveRun(run)) return Math.max(0, now - start)
  return run.durationSeconds != null && run.endedAt ? run.durationSeconds * 1000 : null
}

function percentile(sorted: number[], p: number): number | null {
  if (sorted.length === 0) return null
  const rank = Math.ceil((p / 100) * sorted.length) - 1
  return sorted[Math.min(sorted.length - 1, Math.max(0, rank))]
}

export type RunSummary = {
  /** Runs in the window. */
  total: number
  /** Runs that reached `completed` or `failed`. Cancelled runs are excluded from the rate. */
  finished: number
  succeeded: number
  failed: number
  /** `succeeded / finished`, or null before anything has finished. */
  successRate: number | null
  /** Median and p95 wall time of `completed` runs, in ms. */
  p50Ms: number | null
  p95Ms: number | null
  lastSuccess: PipelineRun | null
  lastFailure: PipelineRun | null
  /** How many of the newest runs share the newest run's status. */
  streak: { status: EntityStatus; count: number } | null
}

/** `runs` newest first, as the API returns them. */
export function summarizeRuns(runs: readonly PipelineRun[]): RunSummary {
  const succeededRuns = runs.filter((r) => r.status === "completed")
  const failedRuns = runs.filter((r) => r.status === "failed")
  const finished = succeededRuns.length + failedRuns.length
  const durations = succeededRuns
    .map((r) => runDurationMs(r))
    .filter((d): d is number => d !== null)
    .sort((a, b) => a - b)

  let streak: RunSummary["streak"] = null
  if (runs.length > 0) {
    const status = runs[0].status
    let count = 0
    while (count < runs.length && runs[count].status === status) count++
    streak = { status, count }
  }

  return {
    total: runs.length,
    finished,
    succeeded: succeededRuns.length,
    failed: failedRuns.length,
    successRate: finished === 0 ? null : succeededRuns.length / finished,
    p50Ms: percentile(durations, 50),
    p95Ms: percentile(durations, 95),
    lastSuccess: succeededRuns[0] ?? null,
    lastFailure: failedRuns[0] ?? null,
    streak,
  }
}
