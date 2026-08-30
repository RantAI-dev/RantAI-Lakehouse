-- Phase 2, Task 2.4: demo/seed query-studio data.
--
-- The fixtures from `src/services/mock/queries.ts`'s `listSaved` and
-- `collaborationStore`, so the console's Saved Queries / Collaboration
-- screens are not empty the moment the mock adapter stops being reachable.
-- Fixed UUIDs + `ON CONFLICT DO NOTHING` (matched via each table's primary
-- key) make this idempotent, same convention as `0002_seed_identity.sql` /
-- `0004_seed_governance.sql` — and split into its own file for the same
-- reason those are: schema (`0005_queries.sql`) and seed data have
-- different lifecycles, and a pure-`INSERT` seed file can be re-run by hand
-- (see `lakehouse-store/tests/queries.rs`) without colliding with a
-- `CREATE TABLE` alongside it. `query_history` is NOT seeded here: it is
-- populated organically by real query runs (`routes::query::run`), and
-- there is no fixture for it to seed from — the mock's `listHistory` rows
-- were never meant to be durable data.
INSERT INTO saved_query (id, title, sql, owner, tags, created_at, updated_at)
VALUES
    ('99999999-9999-4999-8999-000000000001', 'Revenue by region',
     'SELECT region, sum(amount) FROM gold.revenue GROUP BY region',
     'Rina Wijaya', ARRAY['finance', 'gold'],
     now() - interval '1 second', now() - interval '120 minutes'),
    ('99999999-9999-4999-8999-000000000002', 'Hot + cold customer join',
     'SELECT c.id, h.orders FROM hot.customers c JOIN lake.orders_history h ON c.id = h.customer_id LIMIT 100',
     'Bayu Pratama', ARRAY['federated'],
     now() - interval '2 seconds', now() - interval '400 minutes')
ON CONFLICT DO NOTHING;

INSERT INTO collaboration_project (id, name, members, description, created_at, updated_at)
VALUES
    ('aaaaaaaa-aaaa-4aaa-8aaa-000000000001', 'Finance analytics workspace', 8,
     'Shared revenue and collections queries.',
     now() - interval '1 second', now() - interval '60 minutes'),
    ('aaaaaaaa-aaaa-4aaa-8aaa-000000000002', 'Risk investigation', 5,
     'Fraud and credit risk collaborative notebooks.',
     now() - interval '2 seconds', now() - interval '200 minutes')
ON CONFLICT DO NOTHING;
