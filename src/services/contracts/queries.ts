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
  cacheEligible: boolean
  freshnessLagSeconds: number
  policyObligations: string[]
  sources: string[]
  plan: QueryPlanStage[]
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
    cacheHit: boolean
    pushdowns: string[]
    policyObligations: string[]
  }
  plan: QueryPlanStage[]
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
