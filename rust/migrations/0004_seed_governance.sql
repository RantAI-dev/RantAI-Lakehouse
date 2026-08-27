-- Phase 2, Task 2.3: demo/seed governance data.
--
-- The fixtures from `src/services/mock/governance.ts`, so the console's
-- Policies/Quality/Classification/Residency "author" screens are not empty
-- the moment the mock adapter stops being reachable. Fixed UUIDs +
-- `ON CONFLICT DO NOTHING` (matched via each table's primary key) make this
-- idempotent, same convention as `0002_seed_identity.sql`. Split into its
-- own migration file, separate from `0003_governance.sql`'s `CREATE TABLE`s,
-- for the same reason `0001_init.sql`/`0002_seed_identity.sql` are split:
-- schema and seed data have different lifecycles, and a seed file that is
-- only `INSERT`s can be re-executed by hand (see
-- `lakehouse-store/tests/governance.rs`'s idempotency test) without ever
-- hitting a "relation already exists" error from a `CREATE TABLE` alongside
-- it.
INSERT INTO policy (id, name, status, kind, subjects, resources, effect, version, owner, created_at)
VALUES
    ('55555555-5555-4555-8555-000000000001', 'tenant_row_filter_default', 'ready', 'Row filter',     'All analysts',           'tenant-scoped tables',        'Permit with obligation', 3, 'Security',              now() - interval '1 second'),
    ('55555555-5555-4555-8555-000000000002', 'agent_l3_write_gate',       'ready', 'Agent autonomy', 'Digital employees L3',   'Non-critical write targets',  'Require approval',       1, 'Platform Governance',   now() - interval '2 seconds')
ON CONFLICT DO NOTHING;

INSERT INTO quality_rule (id, name, asset, dimension, threshold, severity, last_status, last_run_at, created_at)
VALUES
    ('66666666-6666-4666-8666-000000000001', 'email_verified_completeness', 'serving.mart_customer_segment', 'completeness', '>= 95%',              'medium', 'warning', now() - interval '60 minutes', now() - interval '1 second'),
    ('66666666-6666-4666-8666-000000000002', 'orders_uniqueness',           'silver.orders_enriched',        'uniqueness',   'payment_id unique',   'high',   'passed',  now() - interval '20 minutes', now() - interval '2 seconds')
ON CONFLICT DO NOTHING;

INSERT INTO classification_rule (id, asset, column_name, classification, confidence, review_status, masking_rule, created_at)
VALUES
    ('77777777-7777-4777-8777-000000000001', 'core.customer.customer_360',   'email',       'confidential', 0.98, 'reviewed',     'hash_email',  now() - interval '1 second'),
    ('77777777-7777-4777-8777-000000000002', 'core.commerce.orders_enriched','card_last4',  'restricted',   0.91, 'needs-review', 'show_last4',  now() - interval '2 seconds')
ON CONFLICT DO NOTHING;

INSERT INTO residency_rule (id, tenant, classification, approved_sites, cross_site_allowed, allowed_output, violations_7d, created_at)
VALUES
    ('88888888-8888-4888-8888-000000000001', 'meridian-group', 'restricted',   ARRAY['Jakarta on-prem'],                    false, 'Aggregates only after declassification', 1, now() - interval '1 second'),
    ('88888888-8888-4888-8888-000000000002', 'meridian-group', 'confidential', ARRAY['Jakarta on-prem', 'Singapore (SG)'],  true,  'Masked rows may cross sites',            0, now() - interval '2 seconds')
ON CONFLICT DO NOTHING;
