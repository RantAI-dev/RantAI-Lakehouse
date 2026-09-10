import type { Measured } from "@/lib/measured"
import type { EngineCategory, EntityStatus, WorkloadClass } from "@/lib/status"

export type SavedQuery = {
  id: string
  title: string
  sql: string
  owner: string
  updatedAt: string
  tags: string[]
}

export type QueryHistoryItem = {
  id: string
  sql: string
  user: string
  at: string
  status: Extract<EntityStatus, "completed" | "failed" | "cancelled" | "blocked">
  durationMs: number
  scannedBytes: number
  costUnits: number
  workloadClass: WorkloadClass
  engine: EngineCategory
  cacheAssisted: boolean
  auditEventId?: string
}

/** Simple federated / multi-source execution plan stage for UI. */
export type QueryPlanStage = {
  id: string
  label: string
  location: string
  operation: string
  estimatedBytes?: number
  status?: Extract<EntityStatus, "completed" | "running" | "failed" | "blocked">
}

export type QueryEstimate = {
  estimatedBytes: number
  estimatedCostMin: number
  estimatedCostMax: number
  workloadClass: WorkloadClass
  engine: EngineCategory
  /**
   * `EXPLAIN ESTIMATE` doesn't report cache eligibility — null until a
   * real cache-eligibility check exists.
   */
  cacheEligible: boolean | null
  /** No freshness-lag measurement exists yet. */
  freshnessLagSeconds: Measured
  /** No policy engine exists yet — WS7 builds one. */
  policyObligations: string[] | null
  sources: string[]
  /**
   * WS1 task 1.6: the old plan marked stages that had not run as
   * `status: "completed"`. Null until a real plan exists.
   */
  plan: QueryPlanStage[] | null
}

export type QueryResult = {
  id: string
  columns: string[]
  rows: Record<string, string>[]
  metrics: {
    durationMs: number
    scannedBytes: number
    costUnits: number
    engine: EngineCategory
    workloadClass: WorkloadClass
    /** ClickHouse's response carries no cache-hit flag — not measured. */
    cacheHit: boolean | null
    /** No pushdown computation exists — not measured. */
    pushdowns: string[] | null
    /** No policy engine exists yet — WS7 builds one. */
    policyObligations: string[] | null
  }
  /**
   * WS1 task 1.6: the old plan was a single hardcoded stage claiming
   * "scan + aggregate" for every query. Null until a real plan exists.
   */
  plan: QueryPlanStage[] | null
  auditEventId?: string
}

export type CollaborationProject = {
  id: string
  name: string
  members: number
  updatedAt: string
  description: string
}

export type CreateCollaborationProjectInput = {
  name: string
  collaborators: string[]
  description?: string
}

export interface QueryService {
  listSaved(signal?: AbortSignal): Promise<SavedQuery[]>
  listHistory(signal?: AbortSignal): Promise<QueryHistoryItem[]>
  estimate(sql: string, signal?: AbortSignal): Promise<QueryEstimate>
  run(sql: string, signal?: AbortSignal): Promise<QueryResult>
  generateSql(question: string, signal?: AbortSignal): Promise<{ sql: string; explanation: string; assumptions: string[] }>
  listCollaboration(signal?: AbortSignal): Promise<CollaborationProject[]>
  createCollaborationProject(
    input: CreateCollaborationProjectInput,
    signal?: AbortSignal
  ): Promise<CollaborationProject>
}
