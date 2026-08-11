import { mockCall, stableHash } from "../transport"
import { agoIso } from "./mock-time"
import type { KnowledgeService, SearchHit } from "../contracts/knowledge"

export const mockKnowledgeService: KnowledgeService = {
  listSources(signal) {
    return mockCall(
      () => [
        {
          id: "ks-credit-policy",
          name: "credit-policy-2026",
          kind: "object-storage" as const,
          status: "ready" as const,
          owner: "Risk Analytics",
          version: "v12",
          lastRefresh: agoIso(40),
          chunkCount: 1842,
          embeddingModel: "text-embed-3-large",
          indexStatus: "ready",
          freshnessLagSeconds: 55,
          classification: "confidential" as const,
          dependentAgents: 3,
        },
        {
          id: "ks-faq",
          name: "product-faq",
          kind: "web" as const,
          status: "running" as const,
          owner: "Support AI",
          version: "v4",
          lastRefresh: agoIso(5),
          chunkCount: 420,
          embeddingModel: "text-embed-3-small",
          indexStatus: "indexing",
          freshnessLagSeconds: 120,
          classification: "internal" as const,
          dependentAgents: 2,
        },
      ],
      { signal }
    )
  },
  listVectorJobs(signal) {
    return mockCall(
      () => [
        {
          id: "vj-faq",
          name: "faq_embedding_refresh",
          status: "scheduled" as const,
          source: "knowledge.faq_articles",
          embeddingModel: "text-embed-3-small",
          indexType: "HNSW + BM25",
          lastRunAt: agoIso(180),
          owner: "Support AI",
        },
        {
          id: "vj-policy",
          name: "policy_reindex",
          status: "completed" as const,
          source: "s3://docs/credit-policy/",
          embeddingModel: "text-embed-3-large",
          indexType: "HNSW",
          lastRunAt: agoIso(40),
          owner: "Risk Analytics",
        },
      ],
      { signal }
    )
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
          strategy,
          version: "v12",
          freshnessLagSeconds: 55,
        },
        {
          id: "hit-2",
          title: "Collections playbook FAQ",
          snippet: "Recommended agent actions for L3 autonomy with approval gates…",
          score: 0.84,
          source: "product-faq",
          strategy,
          version: "v4",
          freshnessLagSeconds: 120,
        },
      ]
      return hits
    }, { signal, delayMs: 450 })
  },
}
