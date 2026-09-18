-- WS8 plan Phase B, Task B1: tenant provisioning state.
--
-- MIGRATION NUMBER: the plan text (docs/superpowers/plans/
-- 2026-09-11-ws8-admin-oidc-tenancy.md) names this migration `0040`. By the
-- time this task was implemented, `0040_access_requests.sql` (WS7 plan) and
-- `0041_rescale_seeded_agent_budgets.sql` had already landed on this branch
-- and taken those numbers. This file is `0042`, the next free number
-- (`ls rust/migrations/` verified before writing this file) — content is
-- otherwise exactly what the plan's Task B1 describes.
--
-- WHY: `tenant` (0001_init.sql) already carries `residency` (a free-text
-- policy description — WS8 plan Correction 5, this migration does NOT
-- touch that column, contrary to a literal reading of the grand plan's
-- "Migration 0040 tenant.warehouse_id, tenant.residency"). What it is
-- missing is (a) which Lakekeeper warehouse backs this tenant's Iceberg
-- data, and (b) a resumable checkpoint for the three external calls
-- `POST /api/identity/tenants` now makes (warehouse create, grants,
-- namespace create) — before this migration, none of that was tracked at
-- all, so a crash between steps left no record of what had already
-- happened, forcing a human to inspect Lakekeeper by hand. See WS8 plan
-- Correction 7 for the full state-machine rationale.
--
-- `warehouse_id` is TEXT, not UUID: Lakekeeper's own warehouse ids are its
-- internal opaque identifiers, not necessarily UUIDv4-shaped, and this
-- column stores exactly what Lakekeeper's `management/v1/warehouse`
-- response returns (matching how `lakekeeper-authz-init`'s own `jq -r
-- '.warehouses[]?...id'` treats it — a string, never parsed as a UUID).
--
-- Additive only; nothing here alters a previously-applied statement.
ALTER TABLE tenant
    ADD COLUMN warehouse_id        TEXT,
    ADD COLUMN provisioning_status TEXT NOT NULL DEFAULT 'pending',
    ADD CONSTRAINT tenant_provisioning_status_check
        CHECK (provisioning_status IN (
            'pending', 'warehouse_ready', 'grants_ready',
            'namespace_ready', 'complete', 'not_applicable', 'failed'
        ));

-- Every tenant seeded before this migration existed
-- (rust/migrations/0002_seed_identity.sql's four tenant rows, referred to
-- here only by seed id/file per AGENTS.md rule 12) predates Lakekeeper
-- provisioning entirely and has no warehouse of its own.
-- P2 fix: 'complete' would fabricate a completion that never happened
-- (judge review — nothing was provisioned, so nothing "completed");
-- 'pending' would make every existing tenant look mid-provisioning
-- forever, which is equally false (nothing is IN PROGRESS either).
-- 'not_applicable' is the honest third state: these rows predate
-- provisioning and are not on the state machine at all. Task B4's resume
-- logic (`provisioning_status == "pending"`, `"warehouse_ready"`, ...)
-- never matches 'not_applicable', so a grandfathered tenant is never
-- silently pulled into a provisioning attempt by a later call.
UPDATE tenant SET provisioning_status = 'not_applicable' WHERE warehouse_id IS NULL;

-- WS8 plan Correction 6: `connector`/`pipeline_definition` gain a nullable
-- tenant_id so Phase C's list routes have a real column to filter on. NULL
-- means "not yet assigned to a tenant" and is invisible to every
-- tenant-scoped read once Phase C's enforcement is on (fail closed — an
-- unassigned row shows to nobody, never to everybody).
ALTER TABLE connector ADD COLUMN tenant_id UUID REFERENCES tenant(id) ON DELETE SET NULL;
ALTER TABLE pipeline_definition ADD COLUMN tenant_id UUID REFERENCES tenant(id) ON DELETE SET NULL;

-- P2 fix (seed data verified before writing this statement):
-- rust/migrations/0022_prune_connector_seed.sql seeds exactly the two
-- connector rows below, and no other connector id is reliably present on
-- every branch this migration could apply to (0014's 28-row fixture is
-- deleted by that same file). Backfill ONLY those two rows, matched on
-- (id, name) so a real deployment that reused either id for its own
-- connector (a different `name`) is left NULL rather than silently
-- reassigned — the same (id, name) precision
-- 0027_prune_seeded_activity.sql already established as this codebase's
-- convention for "touch only the exact seeded row, never a broader
-- match." Guarded by EXISTS so this migration does not abort with a
-- foreign-key violation on a database where the seed tenant
-- (0002_seed_identity.sql, id '11111111-1111-4111-8111-000000000001')
-- was deleted — in that case both rows are simply left NULL, which is
-- correct (there is no longer a tenant to assign them to) rather than a
-- migration failure that blocks every later migration behind it.
UPDATE connector
SET tenant_id = '11111111-1111-4111-8111-000000000001'
WHERE (id, name) IN (
    ('conn-pg-lakehouse', 'Lakehouse OLTP (Postgres)'),
    ('conn-s3-warehouse', 'Lakehouse warehouse (RustFS S3)')
)
AND tenant_id IS NULL
AND EXISTS (SELECT 1 FROM tenant WHERE id = '11111111-1111-4111-8111-000000000001');

-- pipeline_definition: NO backfill statement here. Verified (this task's
-- preamble, and re-verified against rust/migrations/0027_prune_seeded_
-- activity.sql on this branch before writing this file):
-- 0027_prune_seeded_activity.sql (WS1 finding J17) already deletes every
-- row 0008_seed_pipelines.sql seeded ('pl-orders-rollup',
-- 'pl-erp-inventory', 'pl-policy-docs', 'pl-embed-faq'), matched by
-- (id, name) exactly the same way. On every branch where WS1 has landed
-- (this plan's own baseline dependency, confirmed present here by
-- 0027's existence), pipeline_definition has zero seeded rows at the
-- point 0042 applies — writing an UPDATE against a table this migration
-- knows has no matching rows would be dead code asserting nothing; every
-- pipeline_definition row from this point forward starts with tenant_id
-- NULL (invisible until assigned via Task C7's
-- `PUT /api/pipelines/{id}/tenant`) unless a future INSERT sets it
-- explicitly.

CREATE INDEX connector_tenant_id_idx ON connector (tenant_id);
CREATE INDEX pipeline_definition_tenant_id_idx ON pipeline_definition (tenant_id);
