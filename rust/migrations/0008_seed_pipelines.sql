-- Phase 2, Task 2.5: demo/seed pipeline definitions.
--
-- The four fixtures from `src/services/mock/pipelines.ts`, so the
-- Pipelines screen isn't empty the moment `mockPipelineService` stops
-- being reachable, same convention as `0004_seed_governance.sql`/
-- `0006_seed_queries.sql`. Fixed ids + `ON CONFLICT DO NOTHING` (matched
-- via the primary key) make this idempotent.
INSERT INTO pipeline_definition (id, name, kind, status, owner, source, target, connector_id, source_asset_id, target_asset_id, schedule, last_run_at, next_run_at, sla_ok, freshness_lag_seconds, created_at)
VALUES
    ('pl-orders-rollup',  'orders_hourly_rollup',  'incremental', 'ready',   'Data Platform',    'core.sales.orders_events',                'core.sales.orders_hourly',  'conn-pg-core',  'tbl-orders-events',       'tbl-orders-enriched',   'Every hour',      now() - interval '4 hours',  now() + interval '56 minutes', true,  120,   now() - interval '1 second'),
    ('pl-erp-inventory',  'erp_inventory_sync',    'batch',       'failed',  'Bayu Pratama',     'ERP Inventory API',                       'bronze.inventory_snapshot', NULL,             'ext-legacy-warehouse',    'tbl-inventory-snapshot','Daily 02:00',     now() - interval '48 hours', NULL,                            false, 86400, now() - interval '2 seconds'),
    ('pl-policy-docs',    'supplier_docs_ingest',  'document',    'running', 'Risk Analytics',   's3://meridian-docs/supplier-policy/',     'ai.supplier_doc_chunks',    'conn-s3-docs',   NULL,                      'kn-supplier-policy',    'On object create', now() - interval '2 hours',  NULL,                            true,  40,    now() - interval '3 seconds'),
    ('pl-embed-faq',      'faq_embedding_refresh', 'vector',      'scheduled','Support AI',      'knowledge.faq_articles',                  'ai.faq_vectors',            NULL,             'kn-supplier-policy',      'vec-support-kb',        'Every 6 hours',   now() - interval '180 minutes', now() + interval '180 minutes', true, 10800, now() - interval '4 seconds')
ON CONFLICT DO NOTHING;
