import type {
  ActorKind,
  AlertStatus,
  Severity,
  StorageTier,
} from "@/lib/status"
import type { Measured } from "@/lib/measured"

/**
 * Executive-operational summary shown on the Overview page.
 *
 * WS1 task 1.8: most fields below became `Measured` because nothing in the
 * API measures them yet (schedule lateness, cache-hit rate, the policy
 * engine, Postgres-backed approvals, agent-run accounting, service probes —
 * owned by WS5/WS7). `streaming` and `incidents` were dropped rather than
 * nulled: neither was ever measured, and the incidents list had no consumer
 * beyond the now-removed incidents card.
 */
export type OverviewSummary = {
  assetsTotal: number
  staleAssets: number
  assetsByTier: Record<StorageTier, { count: Measured; bytes: Measured }>
  pipelines: { active: number; failed: number; delayed: Measured }
  queries: {
    volume24h: number
    p95Ms: number
    failureRate: number
    cacheAssistRate: Measured
    scannedBytes24h: number
  }
  policyViolations7d: Measured
  pendingApprovals: Measured
  agents: { activeRuns: Measured; budgetUsedRate: Measured }
  services: { healthy: Measured; degraded: Measured; unhealthy: Measured }
}

export type ActivityCategory =
  | "pipeline"
  | "query"
  | "schema"
  | "policy"
  | "connector"
  | "agent"
  | "approval"
  | "incident"

export type ActivityItem = {
  id: string
  at: string
  actor: string
  actorKind: ActorKind
  action: string
  target: string
  targetHref?: string
  category: ActivityCategory
  /** Correlates the activity item with its immutable audit event. */
  auditEventId?: string
}

export type AlertItem = {
  id: string
  title: string
  /**
   * `null` when the firing rule was saved with no severity (WS5 item C1:
   * a fired instance copies the rule's own severity verbatim, never
   * invented from the rule's kind) — the backend always sends this key,
   * so `null` here means "genuinely unset," distinct from a field the
   * frontend hasn't loaded yet.
   */
  severity: Severity | null
  source: string
  affected: string
  status: AlertStatus
  assignee?: string
  at: string
  detail: string
  resolutionNote?: string
  href?: string
  /** The `console.alert_rule` id that fired this instance, if any. */
  ruleId?: string
  /** When the rule actually fired, ISO 8601 (WS5 item C1). */
  firedAt?: string
  /** Set once this alert (and its rule) is silenced, ISO 8601. */
  silencedUntil?: string
}

export interface OverviewService {
  getSummary(signal?: AbortSignal): Promise<OverviewSummary>
  listActivity(signal?: AbortSignal): Promise<ActivityItem[]>
  listAlerts(signal?: AbortSignal): Promise<AlertItem[]>
  acknowledgeAlert(id: string, signal?: AbortSignal): Promise<AlertItem>
  resolveAlert(id: string, note: string, signal?: AbortSignal): Promise<AlertItem>
  /**
   * `untilMinutes` must be within `1..=43_200` (30 days) — the backend's
   * own range (`lakehouse_alerts`'s silence route) — a value outside it
   * gets a 400, so callers must not surface a UI that can send one.
   */
  silenceAlert(id: string, untilMinutes: number, signal?: AbortSignal): Promise<AlertItem>
}
