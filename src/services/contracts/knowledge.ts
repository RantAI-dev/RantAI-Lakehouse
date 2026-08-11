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
  /** Catalog asset for this knowledge corpus when registered. */
  assetId?: string
  /** Active vector job producing embeddings for this source. */
  vectorJobId?: string
}

export type VectorJob = {
  id: string
  name: string
  status: EntityStatus
  source: string
  /** Knowledge source id when the job indexes a registered source. */
  sourceId?: string
  /** Resulting vector / knowledge asset in the catalog. */
  outputAssetId?: string
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
  sourceId?: string
  assetId?: string
  strategy: SearchStrategy
  version: string
  freshnessLagSeconds: number
  classification?: Classification
}

export type SearchStrategy = "semantic" | "lexical" | "hybrid"

export type CreateKnowledgeSourceInput = {
  name: string
  kind: KnowledgeSourceKind
  embeddingModel: string
  classification: Classification
  owner?: string
}

export type CreateVectorJobInput = {
  name: string
  source: string
  embeddingModel: string
  indexType: string
  owner?: string
}

export interface KnowledgeService {
  listSources(signal?: AbortSignal): Promise<KnowledgeSource[]>
  listVectorJobs(signal?: AbortSignal): Promise<VectorJob[]>
  search(
    query: string,
    strategy: SearchStrategy,
    signal?: AbortSignal
  ): Promise<SearchHit[]>
  createSource(
    input: CreateKnowledgeSourceInput,
    signal?: AbortSignal
  ): Promise<KnowledgeSource>
  createVectorJob(input: CreateVectorJobInput, signal?: AbortSignal): Promise<VectorJob>
}
