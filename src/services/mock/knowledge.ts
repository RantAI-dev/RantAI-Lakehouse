import { mockCall, stableHash } from "../transport"
import { agoIso } from "./mock-time"
import { createStore } from "./mutable-store"
import type {
  CreateKnowledgeSourceInput,
  CreateVectorJobInput,
  KnowledgeService,
  KnowledgeSource,
  SearchHit,
  VectorJob,
} from "../contracts/knowledge"

const sourceStore = createStore<KnowledgeSource>([
  {
    id: "ks-credit-policy",
    name: "credit-policy-2026",
    kind: "object-storage",
    status: "ready",
    owner: "Risk Analytics",
    version: "v12",
    lastRefresh: agoIso(40),
    chunkCount: 1842,
    embeddingModel: "text-embed-3-large",
    indexStatus: "ready",
    freshnessLagSeconds: 55,
    classification: "confidential",
    dependentAgents: 3,
    assetId: "kn-credit-policy",
    vectorJobId: "vj-policy",
  },
  {
    id: "ks-faq",
    name: "product-faq",
    kind: "web",
    status: "running",
    owner: "Support AI",
    version: "v4",
    lastRefresh: agoIso(5),
    chunkCount: 420,
    embeddingModel: "text-embed-3-small",
    indexStatus: "indexing",
    freshnessLagSeconds: 120,
    classification: "internal",
    dependentAgents: 2,
    assetId: "vec-support-kb",
    vectorJobId: "vj-faq",
  },
])

const vectorStore = createStore<VectorJob>([
  {
    id: "vj-faq",
    name: "faq_embedding_refresh",
    status: "scheduled",
    source: "product-faq",
    sourceId: "ks-faq",
    outputAssetId: "vec-support-kb",
    embeddingModel: "text-embed-3-small",
    indexType: "HNSW + BM25",
    lastRunAt: agoIso(180),
    owner: "Support AI",
  },
  {
    id: "vj-policy",
    name: "policy_reindex",
    status: "completed",
    source: "credit-policy-2026",
    sourceId: "ks-credit-policy",
    outputAssetId: "kn-credit-policy",
    embeddingModel: "text-embed-3-large",
    indexType: "HNSW",
    lastRunAt: agoIso(40),
    owner: "Risk Analytics",
  },
])

export const mockKnowledgeService: KnowledgeService = {
  listSources(signal) {
    return mockCall(() => sourceStore.list(), { signal })
  },
  listVectorJobs(signal) {
    return mockCall(() => vectorStore.list(), { signal })
  },
  search(query, strategy, signal) {
    return mockCall(() => {
      const h = stableHash(`${strategy}:${query}`)
      const hits: SearchHit[] = [
        {
          id: "hit-1",
          title: "Late payment escalation policy",
          snippet: `Section 4.2 covers escalation after ${query || "delinquency"} thresholds…`,
          score: 0.91 - (h % 10) / 100,
          source: "credit-policy-2026",
          sourceId: "ks-credit-policy",
          assetId: "kn-credit-policy",
          strategy,
          version: "v12",
          freshnessLagSeconds: 55,
          classification: "confidential",
        },
        {
          id: "hit-2",
          title: "Collections playbook FAQ",
          snippet: "Recommended agent actions for L3 autonomy with approval gates…",
          score: 0.84,
          source: "product-faq",
          sourceId: "ks-faq",
          assetId: "vec-support-kb",
          strategy,
          version: "v4",
          freshnessLagSeconds: 120,
          classification: "internal",
        },
      ]
      return hits
    }, { signal, delayMs: 450 })
  },
  createSource(input: CreateKnowledgeSourceInput, signal) {
    return mockCall(
      () =>
        sourceStore.prepend({
          id: `ks-${Date.now().toString(36)}`,
          name: input.name,
          kind: input.kind,
          status: "draft",
          owner: input.owner ?? "Current user",
          version: "v1",
          lastRefresh: agoIso(0),
          chunkCount: 0,
          embeddingModel: input.embeddingModel,
          indexStatus: "indexing",
          freshnessLagSeconds: 0,
          classification: input.classification,
          dependentAgents: 0,
        }),
      { signal, delayMs: 400 }
    )
  },
  createVectorJob(input: CreateVectorJobInput, signal) {
    return mockCall(
      () =>
        vectorStore.prepend({
          id: `vj-${Date.now().toString(36)}`,
          name: input.name,
          status: "draft",
          source: input.source,
          embeddingModel: input.embeddingModel,
          indexType: input.indexType,
          lastRunAt: agoIso(0),
          owner: input.owner ?? "Current user",
        }),
      { signal, delayMs: 400 }
    )
  },
}
