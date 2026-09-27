/**
 * Reading a run's event log: level ranking, filtering, the offset from the
 * run's start, and the lines that explain a failure.
 *
 * The run record (`PipelineRun`) carries no error text: `run_to_json`
 * sends no `error` for a Dagster run. The reason a run failed therefore
 * lives only in its log, as `ERROR` events. `failureLines` picks those out
 * so the inspector can show the engine's own words. It never writes a
 * summary of its own.
 */
import type { PipelineRunLogsPage } from "@/services/contracts/pipelines"

export type LogLine = PipelineRunLogsPage["lines"][number]

export type LogLevelFilter = "all" | "info" | "warning" | "error"

const RANK: Record<string, number> = {
  DEBUG: 0,
  INFO: 1,
  WARNING: 2,
  WARN: 2,
  ERROR: 3,
  CRITICAL: 4,
}

export function levelRank(level: string): number {
  return RANK[level.toUpperCase()] ?? 1
}

const THRESHOLD: Record<LogLevelFilter, number> = {
  all: 0,
  info: 1,
  warning: 2,
  error: 3,
}

export type LogFilter = {
  level: LogLevelFilter
  /** Only this step's lines; null for every line, run-level ones included. */
  stepKey: string | null
  query: string
}

/**
 * The lines to show. Lines with an empty message are dropped: the
 * orchestrator emits bare lifecycle markers with no text, which only add
 * blank rows. The caller reports how many were hidden.
 */
export function filterLogLines(lines: readonly LogLine[], filter: LogFilter): LogLine[] {
  const min = THRESHOLD[filter.level]
  const q = filter.query.trim().toLowerCase()
  return lines.filter(
    (line) =>
      line.message.trim() !== "" &&
      levelRank(line.level) >= min &&
      (filter.stepKey === null || line.stepKey === filter.stepKey) &&
      (q === "" || line.message.toLowerCase().includes(q))
  )
}

/** Generic lifecycle errors the engine adds after the real one. */
const GENERIC_FAILURE = /^Execution of (step|run) .* failed\b/

/**
 * The `ERROR` lines that say why a run failed, most specific first: an
 * op's own error message comes before the engine's generic "Execution of
 * step … failed." A run whose log has only generic lines still gets those
 * rather than nothing.
 */
export function failureLines(lines: readonly LogLine[], max = 3): LogLine[] {
  const errors = lines.filter((l) => levelRank(l.level) >= 3 && l.message.trim() !== "")
  const specific = errors.filter((l) => !GENERIC_FAILURE.test(l.message))
  return (specific.length > 0 ? specific : errors).slice(0, max)
}

/** "+00:03.214" from the run's start, "+1:02:03.000" past an hour. */
export function formatLogOffset(ts: number, originMs: number): string {
  const total = Math.max(0, Math.round(ts - originMs))
  const millis = total % 1000
  const secs = Math.floor(total / 1000) % 60
  const mins = Math.floor(total / 60_000) % 60
  const hours = Math.floor(total / 3_600_000)
  const base = `${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}.${String(millis).padStart(3, "0")}`
  return hours > 0 ? `+${hours}:${base}` : `+${base}`
}

/** Plain-text export of a log, for copying or downloading. */
export function logsToText(lines: readonly LogLine[]): string {
  return lines
    .filter((l) => l.message.trim() !== "")
    .map(
      (l) =>
        `${new Date(l.ts).toISOString()} ${l.level.padEnd(7)} ${l.stepKey ?? "-"} ${l.message}`
    )
    .join("\n")
}
