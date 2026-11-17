-- Query history: a run records the tenant it ran under.
--
-- WHY: `query_history` had no tenant column (`0005_queries.sql`,
-- `0046_query_ownership.sql`, `0111_query_history_detail.sql`), so the
-- usage view (`OPS-8`) that sums runs per tenant had nothing to group
-- by. A sum over the whole table would show one organisation's people
-- and queries to another, the same class of leak `SEC-20` closed for
-- saved queries (`0099_saved_query_tenant.sql`). Every new row carries
-- the tenant first, so the view that reads it can be scoped from its
-- first day.
--
-- The column is NULLABLE, and there is no backfill: a row written
-- before this migration may belong to any tenant, because nobody
-- recorded which one the run acted as. Attributing those rows would be
-- a guess, and a guess that decides whose usage a sum shows is worse
-- than the gap it leaves. A new row also stays NULL for a caller that
-- belongs to no tenant (a service identity). Who may read a NULL row
-- is the usage view's rule (only a caller holding `*:*`), written with
-- that view. It is NOT the saved-query rule of `0099` that gives
-- tenant-less rows to the catalog's tenant: a saved query with no
-- tenant is the seeded catalog's, while a run with no tenant is
-- unknown.
--
-- `ON DELETE SET NULL`, the same treatment `saved_query.tenant_id` and
-- the other tenant-scoped columns get: a tenant that goes must not
-- leave rows behind that block its deletion
-- (`lakehouse_store::identity`'s tenant delete). The rows it leaves
-- fall back to the no-tenant rule above.
ALTER TABLE query_history ADD COLUMN tenant_id UUID REFERENCES tenant(id) ON DELETE SET NULL;

-- The usage view reads "one tenant's runs in a time window", so the
-- tenant comes first and the time follows it, newest first.
CREATE INDEX query_history_tenant_at_idx
    ON query_history (tenant_id, at DESC);
