-- WS3 (grand plan §5): any connector row that carries an `adapter` can be
-- ingested by a generated Dagster job (`ingest_factory.py`). Additive:
-- every column is nullable or defaults to an empty JSON value, so every
-- existing connector row (including the two `0022_prune_connector_seed.sql`
-- seeds) keeps parsing with adapter = NULL ("not ingestible yet") until
-- 0034 seeds them. `dial` NEVER carries a credential -- see
-- docs/adr/0013-registry-driven-ingestion.md.
ALTER TABLE connector
  ADD COLUMN adapter        TEXT,
  ADD COLUMN ingest_mode    TEXT,
  ADD COLUMN dial           JSONB NOT NULL DEFAULT '{}'::jsonb,
  ADD COLUMN source_objects JSONB NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN schedule_cron  TEXT,
  ADD CONSTRAINT connector_adapter_check
    CHECK (adapter IS NULL OR adapter IN ('sql','cdc','files','rest','sheets')),
  ADD CONSTRAINT connector_ingest_mode_check
    CHECK (ingest_mode IS NULL OR ingest_mode IN ('batch','cdc'));

-- WS3 plan review X7: a service identity's `scopes`
-- parse into the SAME PermissionSet a role's `permissions` do
-- (`lakehouse_auth::service_token::verify_service_token`,
-- `rust/crates/lakehouse-auth/src/service_token.rs:143`), so
-- `GET /api/connectors/{id}/ingest-spec` can be gated on a real, narrow
-- permission (`ingest:read`) rather than reusing `connector:manage` for a
-- read-only Dagster caller (Phase G's ingest service identity) or adding a
-- second `Policy` variant. Grant it to the seeded Data Engineer role too,
-- so an interactive Data Engineer session (which already holds
-- `connector:manage`, a DIFFERENT resource:action pair that does not
-- satisfy `ingest:read` under `PermissionSet::has`'s exact resource+action
-- match, `lakehouse-auth/src/permissions.rs:90-97`) can read an
-- ingest-spec without also being handed PUT-level `connector:manage`.
--
-- Keyed on the token's absence, following `0020_extend_role_grants.sql`'s
-- own guard, so re-running never appends `ingest:read` twice -- not on the
-- exact current permission string, which several other migrations
-- (`0020_extend_role_grants.sql`) already append to, and any deployment
-- that has hand-edited the row would otherwise silently skip this grant.
UPDATE role
SET permissions = permissions || ', ingest:read'
WHERE name = 'Data Engineer'
  AND permissions !~ '(^|,)\s*ingest:read\s*(,|$)';
