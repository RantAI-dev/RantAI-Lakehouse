import type { Classification, EntityStatus } from "@/lib/status"

export type KnowledgeSourceKind =
  | "file"
  | "object-storage"
  | "web"
  | "table"
  | "query"
  | "manual"

export type IndexStatus = "ready" | "indexing" | "degraded"

export type KnowledgeSource = {
  id: string
  name: string
  kind: KnowledgeSourceKind
  status: EntityStatus
  owner: string
  version: string
  lastRefresh: string
  chunkCount: number
  embeddingModel: string
  indexStatus: IndexStatus
  freshnessLagSeconds: number
  classification: Classification
  dependentAgents: number
}

export type VectorJob = {
  id: string
  name: string
  status: EntityStatus
  source: string
  embeddingModel: string
  indexType: string
  lastRunAt: string
  owner: string
}

export type SearchHit = {
  id: string
  title: string
  snippet: string
  score: number
  source: string
  strategy: SearchStrategy
  version: string
  freshnessLagSeconds: number
}

export type SearchStrategy = "semantic" | "lexical" | "hybrid"

export interface KnowledgeService {
  listSources(signal?: AbortSignal): Promise<KnowledgeSource[]>
  listVectorJobs(signal?: AbortSignal): Promise<VectorJob[]>
  search(
    query: string,
    strategy: SearchStrategy,
    signal?: AbortSignal
  ): Promise<SearchHit[]>
}
