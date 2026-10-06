import { parseTimestamp } from "@/lib/format"

/**
 * Row shapers for Home's optional cards (sources, open alerts, saved
 * queries). Pure, like `lib/home-pipelines`, so what a card lists can be
 * tested without a page. Each one only orders and caps what a service
 * already returned; nothing is counted or derived beyond that.
 */
function ms(iso: string | null | undefined): number {
  if (!iso) return 0
  const t = parseTimestamp(iso).getTime()
  return Number.isNaN(t) ? 0 : t
}

export type SourceLike = { id: string; name: string; type: string; health: string }

/** Worst first: what needs a look leads, the healthy ones fill the rest. */
const HEALTH_RANK: Record<string, number> = { unhealthy: 0, degraded: 1, unknown: 2, healthy: 3 }

export function sourceRows<T extends SourceLike>(sources: readonly T[], limit = 5): T[] {
  return [...sources]
    .sort(
      (a, b) =>
        (HEALTH_RANK[a.health] ?? 2) - (HEALTH_RANK[b.health] ?? 2) ||
        a.name.localeCompare(b.name),
    )
    .slice(0, limit)
}

export type AlertLike = {
  id: string
  title: string
  severity: string | null
  status: string
  at: string
}

const SEVERITY_RANK: Record<string, number> = { critical: 0, high: 1, medium: 2, low: 3, info: 4 }

/**
 * Open alerts only, most severe first and newest within a severity. An
 * alert saved without a severity sorts after the ones that have one: its
 * rank is unknown, not low.
 */
export function openAlertRows<T extends AlertLike>(alerts: readonly T[], limit = 5): T[] {
  return alerts
    .filter((a) => a.status === "open")
    .sort(
      (a, b) =>
        (a.severity === null ? 5 : (SEVERITY_RANK[a.severity] ?? 5)) -
          (b.severity === null ? 5 : (SEVERITY_RANK[b.severity] ?? 5)) || ms(b.at) - ms(a.at),
    )
    .slice(0, limit)
}

export type SavedQueryLike = { id: string; title: string; updatedAt: string }

/** The most recently changed saved queries first. */
export function savedQueryRows<T extends SavedQueryLike>(queries: readonly T[], limit = 5): T[] {
  return [...queries].sort((a, b) => ms(b.updatedAt) - ms(a.updatedAt)).slice(0, limit)
}

/** Where a saved query opens in Query Studio (the link `Recent` uses). */
export function savedQueryHref(id: string): string {
  return `/query-studio?saved=${encodeURIComponent(id)}`
}

/** A failed read as a card says it: refused for this person, or failed. */
export type CardRead = "loading" | "ok" | "denied" | "error"

export function cardRead(status: string, error: { code: string; status?: number } | null): CardRead {
  if (status === "loading") return "loading"
  if (status !== "error") return "ok"
  return error && (error.code === "permission_denied" || error.status === 403 || error.status === 401)
    ? "denied"
    : "error"
}
