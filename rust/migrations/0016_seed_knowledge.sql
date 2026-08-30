-- Phase 2, Task 2.8: demo/seed knowledge sources and vector jobs, taken
-- from `src/services/mock/knowledge.ts`'s fixtures — same convention as
-- `0014_seed_connectors.sql`. Fixed ids + `ON CONFLICT DO NOTHING` make
-- this idempotent.
INSERT INTO knowledge_source (id, name, kind, status, owner, version, last_refresh, chunk_count, embedding_model, index_status, freshness_lag_seconds, classification, dependent_agents, asset_id, vector_job_id, created_at)
VALUES
    ('ks-supplier-policy', 'logistics-sop-2026', 'object-storage', 'ready', 'Risk Analytics', 'v12', now() - interval '40 minutes', 1842, 'text-embed-3-large', 'ready', 55, 'confidential', 3, 'kn-supplier-policy', 'vj-policy', now()),
    ('ks-faq', 'product-faq', 'web', 'running', 'Support AI', 'v4', now() - interval '5 minutes', 420, 'text-embed-3-small', 'indexing', 120, 'internal', 2, 'vec-support-kb', 'vj-faq', now())
ON CONFLICT DO NOTHING;

INSERT INTO vector_job (id, name, status, source, source_id, output_asset_id, embedding_model, index_type, last_run_at, owner, created_at)
VALUES
    ('vj-faq', 'faq_embedding_refresh', 'scheduled', 'product-faq', 'ks-faq', 'vec-support-kb', 'text-embed-3-small', 'HNSW + BM25', now() - interval '180 minutes', 'Support AI', now()),
    ('vj-policy', 'policy_reindex', 'completed', 'logistics-sop-2026', 'ks-supplier-policy', 'kn-supplier-policy', 'text-embed-3-large', 'HNSW', now() - interval '40 minutes', 'Risk Analytics', now())
ON CONFLICT DO NOTHING;
