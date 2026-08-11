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
  status: Extract<EntityStatus, "completed" | "failed" | "cancelled">
  durationMs: number
  scannedBytes: number
  costUnits: number
  workloadClass: WorkloadClass
  engine: EngineCategory
  cacheAssisted: boolean
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
}

export type QueryResult = {
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
}

export type CollaborationProject = {
  id: string
  name: string
  members: number
  updatedAt: string
  description: string
}

export interface QueryService {
  listSaved(signal?: AbortSignal): Promise<SavedQuery[]>
  listHistory(signal?: AbortSignal): Promise<QueryHistoryItem[]>
  estimate(sql: string, signal?: AbortSignal): Promise<QueryEstimate>
  run(sql: string, signal?: AbortSignal): Promise<QueryResult>
  generateSql(question: string, signal?: AbortSignal): Promise<{ sql: string; explanation: string; assumptions: string[] }>
  listCollaboration(signal?: AbortSignal): Promise<CollaborationProject[]>
}
