import type {
  ActorKind,
  AlertStatus,
  Severity,
  StorageTier,
} from "@/lib/status"

/** Executive-operational summary shown on the Overview page. */
export type OverviewSummary = {
  assetsTotal: number
  staleAssets: number
  assetsByTier: Record<StorageTier, { count: number; bytes: number }>
  pipelines: { active: number; failed: number; delayed: number }
  queries: {
    volume24h: number
    p95Ms: number
    failureRate: number
    cacheAssistRate: number
    scannedBytes24h: number
  }
  policyViolations7d: number
  pendingApprovals: number
  agents: { activeRuns: number; budgetUsedRate: number }
  services: { healthy: number; degraded: number; unhealthy: number }
  incidents: OverviewIncident[]
}

export type OverviewIncident = {
  id: string
  title: string
  severity: Severity
  source: string
  at: string
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
  severity: Severity
  source: string
  affected: string
  status: AlertStatus
  assignee?: string
  at: string
  detail: string
  resolutionNote?: string
  href?: string
}

export interface OverviewService {
  getSummary(signal?: AbortSignal): Promise<OverviewSummary>
  listActivity(signal?: AbortSignal): Promise<ActivityItem[]>
  listAlerts(signal?: AbortSignal): Promise<AlertItem[]>
  acknowledgeAlert(id: string, signal?: AbortSignal): Promise<AlertItem>
  resolveAlert(id: string, note: string, signal?: AbortSignal): Promise<AlertItem>
}
