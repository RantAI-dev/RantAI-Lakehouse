-- Phase 2, Task 2.6: demo/seed storage fixtures, from
-- `src/services/mock/storage.ts`. Fixed ids + `ON CONFLICT DO NOTHING`
-- make this idempotent, same convention as every other seed migration.
INSERT INTO lifecycle_policy (id, name, scope, hot_days, warm_days, cold_after_days, status, estimated_savings, last_applied_at, created_at)
VALUES
    ('lp-1', 'default_analytics_lifecycle', 'core.* analytical tables', 14, 90, 91,  'ready', '68% vs all-hot',                 now() - interval '300 minutes',  now() - interval '1 second'),
    ('lp-2', 'ai_derivative_retention',     'ai.* vector datasets',      0,  0,  365, 'draft', 'Rebuildable from lineage',       now() - interval '1000 minutes', now() - interval '2 seconds')
ON CONFLICT DO NOTHING;

INSERT INTO tiering_op (id, asset, asset_id, from_tier, to_tier, status, at, detail)
VALUES
    ('op-1', 'lake.sales.orders_history',   'ice-orders-history', 'warm', 'cold', 'completed', now() - interval '200 minutes', 'Exported partitions 2024-Q1 → Iceberg snapshot'),
    ('op-2', 'core.sales.orders_events',    'tbl-orders-events',  'hot',  'warm', 'failed',    now() - interval '40 minutes',  'Checksum mismatch on partition 2026-06-01'),
    ('op-3', 'lake.sales.orders_history',   'ice-orders-history', 'cold', 'hot',  'running',   now() - interval '5 minutes',   'Rehydrate Q1 2024 partitions to Hot for investigation')
ON CONFLICT DO NOTHING;
