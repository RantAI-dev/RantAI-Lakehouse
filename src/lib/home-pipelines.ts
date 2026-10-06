import { parseTimestamp } from "@/lib/format"

/**
 * "Pipeline runs" on Home's second screen: events, not metrics. Pure
 * shapers, so what is shown can be tested without a page.
 */
export type PipelineLike = {
  id: string
  name: string
  status: string
  lastRunAt: string | null
  nextRunAt?: string | null
}

export type PipelineRow = {
  id: string
  name: string
  status: string
  lastRunAt: string | null
  nextRunAt: string | null
}

function ms(iso: string | null | undefined): number | null {
  if (!iso) return null
  const t = parseTimestamp(iso).getTime()
  return Number.isNaN(t) ? null : t
}

/**
 * Pipelines that have something to report: a run that happened, or one
 * that is scheduled. A manual pipeline that never ran is left out — there
 * is no event to show. Failed ones lead (they are the news), then the most
 * recently run.
 */
export function pipelineRows(pipelines: PipelineLike[], limit = 5): PipelineRow[] {
  return pipelines
    .filter((p) => ms(p.lastRunAt) !== null || ms(p.nextRunAt) !== null)
    .map((p) => ({
      id: p.id,
      name: p.name,
      status: p.status,
      lastRunAt: ms(p.lastRunAt) === null ? null : p.lastRunAt,
      nextRunAt: ms(p.nextRunAt) === null ? null : (p.nextRunAt ?? null),
    }))
    .sort((a, b) => {
      const af = a.status === "failed" ? 1 : 0
      const bf = b.status === "failed" ? 1 : 0
      if (af !== bf) return bf - af
      return (ms(b.lastRunAt) ?? 0) - (ms(a.lastRunAt) ?? 0)
    })
    .slice(0, limit)
}

/** Time until a future timestamp: "now", "in 12m", "in 3h", "in 2d". Past or unparseable → null. */
export function formatUntil(iso: string | null | undefined, now = Date.now()): string | null {
  const t = ms(iso)
  if (t === null) return null
  const diff = t - now
  if (diff < 0) return null
  const mins = Math.round(diff / 60_000)
  if (mins < 1) return "now"
  if (mins < 60) return `in ${mins}m`
  const hours = Math.round(mins / 60)
  if (hours < 24) return `in ${hours}h`
  return `in ${Math.round(hours / 24)}d`
}
