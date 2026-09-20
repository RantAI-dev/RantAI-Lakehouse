import type { EngineCategory, EntityStatus, WorkloadClass } from "@/lib/status"

export type SavedQuery = {
  id: string
  title: string
  sql: string
  owner: string
  updatedAt: string
  tags: string[]
}

/** One result cell, in whatever type ClickHouse reported it as. */
export type QueryCell = string | number | boolean | null

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
  /** Null when the query could not be planned — see `error`. */
  estimatedBytes: number | null
  estimatedCostMin: number | null
  estimatedCostMax: number | null
  /** Why there is no estimate, in ClickHouse's words. */
  error?: string | null
  workloadClass: WorkloadClass
  engine: EngineCategory
  /** Null when nothing here can tell whether the cache would serve it. */
  cacheEligible: boolean | null
  /** Lag of the stalest source table; null when no source is known. */
  freshnessLagSeconds: number | null
  /** Null when no policy engine was consulted, which is not the same as none. */
  policyObligations: string[] | null
  sources: string[]
  plan: QueryPlanStage[]
}

export type QueryResult = {
  id: string
  columns: string[]
  rows: Record<string, QueryCell>[]
  /** How many rows the query produced, which `rows` may only be part of. */
  rowCount: number
  /** True when `rows` was cut to `rowLimit` before being sent. */
  truncated: boolean
  rowLimit: number
  metrics: {
    durationMs: number
    scannedBytes: number
    costUnits: number
    engine: EngineCategory
    workloadClass: WorkloadClass
    /** Null: nothing here reads ClickHouse's query cache. */
    cacheHit: boolean | null
    /** Null: no optimizer report is read, so "none" would be a guess. */
    pushdowns: string[] | null
    /** Null: no policy engine runs before a query. */
    policyObligations: string[] | null
  }
  plan: QueryPlanStage[]
  auditEventId?: string
}

/** What "Save query" in Query Studio sends. */
export type SaveQueryInput = {
  title: string
  sql: string
  tags: string[]
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
  saveQuery(input: SaveQueryInput, signal?: AbortSignal): Promise<SavedQuery>
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
