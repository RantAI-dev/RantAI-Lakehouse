-- DATA-15 F3: a grant made straight from an asset's page, with no request
-- behind it.
--
-- `access_grant` (0040) only ever held what an approved access request
-- granted: `approval_id` was NOT NULL, nothing recorded who granted, and
-- nothing named the asset the grant was made from. The asset page now lets
-- a holder of `access:approve` grant one of the three requestable
-- permissions to a user directly, and revoke any grant made from that
-- asset. So:
--
-- 1. `approval_id` becomes nullable: a direct grant has no approval.
-- 2. `granted_by_user_id` records who made a direct grant. `ON DELETE SET
--    NULL`, like `revoked_by_user_id`: deleting the admin must not delete
--    or block the grant they made.
-- 3. `catalog_id` records the asset a direct grant was made from (the id the
--    console and the API use, at most 200 characters like
--    `asset_annotation.asset_id`). A grant that came from a request is
--    found through its approval's `resource` instead, so nothing is
--    backfilled and no existing row changes.
-- 4. A row must say where it came from: an approval, or an asset. The
--    plan asked for "an approval or a granter", but the granter column is
--    `ON DELETE SET NULL`, and with that check deleting the admin who made
--    a direct grant would violate it and fail; `catalog_id` is never
--    cleared, so a direct grant stays valid when its granter is deleted.
--
-- The grant still only ever ADDS a permission token at principal-load time
-- (`lakehouse-auth` `load_principal_for_user` reads
-- `revoked_at IS NULL AND expires_at > now()`), which is why a revoke
-- applies at the grantee's next request and needs no change there.

ALTER TABLE access_grant
    ALTER COLUMN approval_id DROP NOT NULL,
    ADD COLUMN granted_by_user_id UUID REFERENCES app_user (id) ON DELETE SET NULL,
    ADD COLUMN catalog_id TEXT,
    ADD CONSTRAINT access_grant_catalog_id_length CHECK (
        catalog_id IS NULL OR char_length(catalog_id) BETWEEN 1 AND 200
    ),
    ADD CONSTRAINT access_grant_has_an_origin CHECK (
        approval_id IS NOT NULL OR catalog_id IS NOT NULL
    );

-- The Grants card lists the active grants of one asset.
CREATE INDEX access_grant_catalog_id_idx ON access_grant (catalog_id)
    WHERE catalog_id IS NOT NULL AND revoked_at IS NULL;
