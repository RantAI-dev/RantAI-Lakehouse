-- Query Studio: a saved query belongs to a tenant.
--
-- WHY: `saved_query` had no tenant column, so every tenant of a
-- deployment read every other tenant's saved queries — their titles,
-- their tags and, above all, their SQL. A saved query is a statement
-- somebody wrote against their own data, and a statement is not a
-- title to be shared: it names tables, columns and filters that the
-- tenant that wrote it may read and the tenant that reads it may not.
--
-- The column is NULLABLE, and there is no backfill: a row written
-- before this migration belongs to no tenant, because nobody recorded
-- which tenant wrote it. Attributing those rows to a tenant would be a
-- guess, and a guess that decides who may read a statement is worse
-- than the gap it closes. The read rule settles them instead — a row
-- with no tenant is read by the tenant that owns the shared catalog
-- (`CATALOG_TENANT_ID`), and by nobody else — so the seeded rows stay
-- usable in the tenant that owns the catalog and leak to no one.
--
-- `ON DELETE SET NULL`, the same treatment the other tenant-scoped
-- columns already get (`file_upload.tenant_id`,
-- `upload_table_claim.tenant_id`, `pipeline_lag_state.tenant_id`): a
-- tenant that goes must not leave rows behind that block its deletion
-- (`lakehouse_store::identity`'s tenant delete). The rows it leaves
-- fall back to the no-tenant rule above.
ALTER TABLE saved_query ADD COLUMN tenant_id UUID REFERENCES tenant(id) ON DELETE SET NULL;

-- Every saved-query read is "one tenant's rows, or the rows with no
-- tenant, newest first", so the tenant comes first and the sort order
-- follows it.
CREATE INDEX saved_query_tenant_updated_at_idx
    ON saved_query (tenant_id, updated_at DESC);
