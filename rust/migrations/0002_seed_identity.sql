-- Phase 2, Task 2.2: demo/seed identity data.
--
-- WHAT THIS IS: the fixtures from `src/services/mock/identity.ts`, moved
-- into Postgres so the console's Identity screens are not empty the moment
-- the in-browser mock is deleted. These are the *same fictional people and
-- companies* the console has displayed since it was built ("Meridian
-- Group", `*@meridian.example`) — `.example` is the RFC 2606 reserved TLD
-- and can never route to a real mailbox, so nothing here can be mistaken
-- for, or used to contact, a real person.
--
-- WHAT THIS IS NOT: it is not authentication data. There is no password,
-- token, or secret in any row below; `service_identity` records that a
-- credential exists (name/scopes/expiry), never the credential itself.
--
-- IDEMPOTENCE: `sqlx::migrate!` records applied versions and never re-runs
-- a migration, but this file does not rely on that — every statement uses a
-- fixed, hardcoded UUID plus `ON CONFLICT DO NOTHING`, so applying it twice
-- (or applying it by hand against a database that already has the rows) is
-- a no-op rather than a duplicate-key failure. Fixed UUIDs also mean the
-- seed rows have stable ids across every environment, which is what makes
-- them safe to reference from a bug report or a manual `DELETE`.
--
-- `created_at` is staggered by a few seconds per row (newest first) because
-- every list query in `lakehouse_store::identity` orders by `created_at
-- DESC` — this reproduces the exact fixture order `mock/identity.ts`
-- presented, so the screens look unchanged after the cutover.

-- ── tenants ─────────────────────────────────────────────────────────────
INSERT INTO tenant (id, name, slug, plan, residency, storage_bytes, quota_compute, used_compute, created_at)
VALUES
    ('11111111-1111-4111-8111-000000000001', 'Meridian Group',           'meridian-group',     'Enterprise', 'Jakarta (ID) + Singapore (SG)', 70368744177664, 20000, 12400, now() - interval '1 second'),
    ('11111111-1111-4111-8111-000000000002', 'Meridian Retail',          'meridian-retail',    'Enterprise', 'Jakarta (ID)',                 23073478443008, 12000,  8850, now() - interval '2 seconds'),
    ('11111111-1111-4111-8111-000000000003', 'Meridian Logistics',       'meridian-logistics', 'Standard',   'Jakarta (ID)',                 13194139533312,  8000,  4200, now() - interval '3 seconds'),
    ('11111111-1111-4111-8111-000000000004', 'Meridian Labs (sandbox)',  'meridian-labs',      'Trial',      'Singapore (SG)',                2199023255552,  2000,   310, now() - interval '4 seconds')
ON CONFLICT DO NOTHING;

-- ── roles ───────────────────────────────────────────────────────────────
INSERT INTO role (id, name, permissions, description, created_at)
VALUES
    ('22222222-2222-4222-8222-000000000001', 'Analyst',          'query:read, catalog:read, lineage:read',    'Read governed data and run approved queries.',              now() - interval '1 second'),
    ('22222222-2222-4222-8222-000000000002', 'Approver',         'agent:approve, policy:review',              'Approve L3 agent actions and policy changes.',              now() - interval '2 seconds'),
    ('22222222-2222-4222-8222-000000000003', 'Governance Admin', 'policy:*, residency:*, audit:read',         'Manage policies, residency, and classifications.',          now() - interval '3 seconds'),
    ('22222222-2222-4222-8222-000000000004', 'Data Engineer',    'pipeline:*, catalog:write, connector:manage','Build and operate pipelines, connectors, and models.',      now() - interval '4 seconds'),
    ('22222222-2222-4222-8222-000000000005', 'Data Scientist',   'query:read, feature:write, notebook:run',   'Explore governed data and build features and models.',      now() - interval '5 seconds'),
    ('22222222-2222-4222-8222-000000000006', 'Platform Admin',   '*:*',                                       'Full platform administration including tenants and quotas.',now() - interval '6 seconds'),
    ('22222222-2222-4222-8222-000000000007', 'Dashboard Viewer', 'dashboard:read',                            'Read published dashboards only; no ad-hoc query access.',   now() - interval '7 seconds')
ON CONFLICT DO NOTHING;

-- ── users ───────────────────────────────────────────────────────────────
-- `last_activity_at` reproduces each fixture's `agoIso(minutes)` offset,
-- relative to when this migration is applied.
INSERT INTO app_user (id, name, email, status, last_activity_at, created_at)
VALUES
    ('33333333-3333-4333-8333-000000000001', 'Rina Wijaya',       'rina@meridian.example',   'active',   now() - interval '9 minutes',     now() - interval '1 second'),
    ('33333333-3333-4333-8333-000000000002', 'Bayu Pratama',      'bayu@meridian.example',   'active',   now() - interval '40 minutes',    now() - interval '2 seconds'),
    ('33333333-3333-4333-8333-000000000003', 'Dewi Anggraini',    'dewi@meridian.example',   'active',   now() - interval '60 minutes',    now() - interval '3 seconds'),
    ('33333333-3333-4333-8333-000000000004', 'Andi Kusuma',       'andi@meridian.example',   'active',   now() - interval '15 minutes',    now() - interval '4 seconds'),
    ('33333333-3333-4333-8333-000000000005', 'Sari Handayani',    'sari@meridian.example',   'active',   now() - interval '4 minutes',     now() - interval '5 seconds'),
    ('33333333-3333-4333-8333-000000000006', 'Fajar Nugroho',     'fajar@meridian.example',  'active',   now() - interval '2 minutes',     now() - interval '6 seconds'),
    ('33333333-3333-4333-8333-000000000007', 'Maya Lestari',      'maya@meridian.example',   'active',   now() - interval '120 minutes',   now() - interval '7 seconds'),
    ('33333333-3333-4333-8333-000000000008', 'Reza Hakim',        'reza@meridian.example',   'active',   now() - interval '28 minutes',    now() - interval '8 seconds'),
    ('33333333-3333-4333-8333-000000000009', 'Putri Ramadhani',   'putri@meridian.example',  'inactive', now() - interval '2880 minutes',  now() - interval '9 seconds'),
    ('33333333-3333-4333-8333-000000000010', 'Hendra Setiawan',   'hendra@meridian.example', 'inactive', now() - interval '10080 minutes', now() - interval '10 seconds'),
    ('33333333-3333-4333-8333-000000000011', 'Citra Amelia',      'citra@meridian.example',  'active',   now() - interval '70 minutes',    now() - interval '11 seconds'),
    ('33333333-3333-4333-8333-000000000012', 'Yoga Prasetya',     'yoga@meridian.example',   'active',   now() - interval '6 minutes',     now() - interval '12 seconds')
ON CONFLICT DO NOTHING;

-- ── role memberships ────────────────────────────────────────────────────
-- Joined by natural key (email / role name) rather than repeating the
-- UUIDs: the pairing is what this table means, and spelling it as
-- ('rina@meridian.example', 'Analyst') is checkable by eye against
-- `mock/identity.ts` in a way two opaque UUIDs are not. The joins also make
-- a typo in a name fail loudly-as-a-missing-row here rather than silently
-- pointing at the wrong person.
INSERT INTO app_user_role (user_id, role_id)
SELECT u.id, r.id
FROM (VALUES
    ('rina@meridian.example',   'Analyst'),
    ('rina@meridian.example',   'Approver'),
    ('bayu@meridian.example',   'Data Engineer'),
    ('dewi@meridian.example',   'Governance Admin'),
    ('andi@meridian.example',   'Data Engineer'),
    ('andi@meridian.example',   'Analyst'),
    ('sari@meridian.example',   'Analyst'),
    ('fajar@meridian.example',  'Platform Admin'),
    ('maya@meridian.example',   'Analyst'),
    ('maya@meridian.example',   'Approver'),
    ('reza@meridian.example',   'Data Scientist'),
    ('putri@meridian.example',  'Analyst'),
    ('hendra@meridian.example', 'Analyst'),
    ('citra@meridian.example',  'Governance Admin'),
    ('citra@meridian.example',  'Approver'),
    ('yoga@meridian.example',   'Data Engineer')
) AS m(email, role_name)
JOIN app_user u ON u.email = m.email
JOIN role r ON r.name = m.role_name
ON CONFLICT DO NOTHING;

-- ── tenant memberships ──────────────────────────────────────────────────
INSERT INTO app_user_tenant (user_id, tenant_id)
SELECT u.id, t.id
FROM (VALUES
    ('rina@meridian.example',   'meridian-group'),
    ('bayu@meridian.example',   'meridian-group'),
    ('bayu@meridian.example',   'meridian-logistics'),
    ('dewi@meridian.example',   'meridian-group'),
    ('dewi@meridian.example',   'meridian-retail'),
    ('dewi@meridian.example',   'meridian-logistics'),
    ('andi@meridian.example',   'meridian-retail'),
    ('sari@meridian.example',   'meridian-retail'),
    ('fajar@meridian.example',  'meridian-group'),
    ('fajar@meridian.example',  'meridian-retail'),
    ('fajar@meridian.example',  'meridian-logistics'),
    ('maya@meridian.example',   'meridian-logistics'),
    ('reza@meridian.example',   'meridian-group'),
    ('reza@meridian.example',   'meridian-retail'),
    ('putri@meridian.example',  'meridian-retail'),
    ('hendra@meridian.example', 'meridian-logistics'),
    ('citra@meridian.example',  'meridian-group'),
    ('yoga@meridian.example',   'meridian-group'),
    ('yoga@meridian.example',   'meridian-logistics')
) AS m(email, tenant_slug)
JOIN app_user u ON u.email = m.email
JOIN tenant t ON t.slug = m.tenant_slug
ON CONFLICT DO NOTHING;

-- ── service identities ──────────────────────────────────────────────────
-- Metadata only — no secret is stored anywhere in this schema. `expires_at`
-- is relative to application time so the rotation statuses stay coherent
-- with the clock (`price-crawler-agent` is genuinely past its expiry and is
-- marked 'expired'; the two 'due' rows expire soonest).
INSERT INTO service_identity (id, name, scopes, environment, rotation_status, expires_at, last_used_at, created_at)
VALUES
    ('44444444-4444-4444-8444-000000000001', 'bi-dashboard-reader',    ARRAY['query:read','catalog:read'],                'production', 'current', now() + interval '30 days', now() - interval '15 minutes',   now() - interval '1 second'),
    ('44444444-4444-4444-8444-000000000002', 'ingestion-worker',       ARRAY['ingest:write','catalog:register'],          'production', 'due',     now() + interval '7 days',  now() - interval '2 minutes',    now() - interval '2 seconds'),
    ('44444444-4444-4444-8444-000000000003', 'dagster-orchestrator',   ARRAY['pipeline:run','catalog:write','query:read'],'production', 'current', now() + interval '60 days', now(),                           now() - interval '3 seconds'),
    ('44444444-4444-4444-8444-000000000004', 'embed-token-signer',     ARRAY['dashboard:embed'],                          'production', 'current', now() + interval '45 days', now() - interval '35 minutes',   now() - interval '4 seconds'),
    ('44444444-4444-4444-8444-000000000005', 'partner-share-exporter', ARRAY['share:write','query:read'],                 'production', 'due',     now() + interval '3 days',  now() - interval '400 minutes',  now() - interval '5 seconds'),
    ('44444444-4444-4444-8444-000000000006', 'price-crawler-agent',    ARRAY['ingest:write'],                             'staging',    'expired', now() - interval '5 days',  now() - interval '7300 minutes', now() - interval '6 seconds')
ON CONFLICT DO NOTHING;
